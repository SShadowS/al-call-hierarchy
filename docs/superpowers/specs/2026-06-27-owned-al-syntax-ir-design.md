# Owned AL Syntax IR — Grammar-Insulation for the AL Analyzer — Design

**Status:** Spec v3 (pre-implementation, **review-final** — 3 review rounds by GPT-5.5 Pro +
Gemini 3.1 Pro Preview; both approved the architecture, round-3 fixes folded in: dropped the
redundant `semantic_slots!` DSL, added order/visit invariants INV-1/2/3, `ts_id` ephemerality,
tiered parse-once dual-run, malformed-AL recovery fixtures, taxonomy gaps try/asserterror/foreach).
Author: engine team. **v3 supersedes v2's hybrid "CST adapter seam."** Under the project mantra — *best solution, time no constraint, any
refactor on the table, nothing released, all downstream consumers ours* — and a second hard
review round by GPT-5.5 Pro + Gemini 3.1 Pro Preview, the chosen end-state is a **fully owned,
grammar-independent AL syntax IR**: tree-sitter parses once, a lowerer projects into the IR,
and the engine **never touches `tree_sitter::Node`, raw fields, or tree-sitter queries.**

> **For agentic workers:** REQUIRED SUB-SKILL — `superpowers:subagent-driven-development`. The
> migration spine is **dual-run parity** (§5): the IR path runs *beside* the existing CST walk
> and must produce identical engine output across the BC corpus before the old path is deleted.
> The al-sem differential goldens (`cargo test`, zero divergences) gate every step, but they are
> necessary-not-sufficient (§8) — IR snapshots, lowerer fixtures, raw-kind coverage, and the
> boundary scanner gate alongside them.

---

## 1. Why

### 1.1 The original sin

This engine was born as a faithful Rust port of al-sem (TS), which mirrored tree-sitter's
**concrete** syntax tree. So the engine treats the CST *as its data model*: ~400+ sites across
~25 files read raw node kinds, field names, and parent/child topology directly. That coupling is
the root cause of every grammar-drift incident.

### 1.2 The v3 grammar post-mortem (the trigger)

tree-sitter-al v3 inserted **wrapper nodes** between containers and their former direct children
(`code_block.body`→`statement_block`, object/field bodies→`declaration_body`, dataitem→
`report_body`, case branch→`case_body`, `var_section`→`var_body`). The empirical law (now in
`CLAUDE.md`):

> **Recursive AST walks survive a grammar bump; flat direct-child iterations break — silently.**

Every flat `named_children(x).find(|c| c.kind()=="…")` returned **zero**, not an error. It zeroed
the L5 transaction detectors (d40/d46/d47/d49/d51), the CFN statement tree, unreachable detection,
the temp-table scan, object-property reads, global-var extraction, statement-position call
classification, and member-trigger resolution — caught only by goldens, only where covered. Fix
was per-site whack-a-mole.

### 1.3 Coupling census (current `master`)

| Surface | Coupling forms (all must count) | ≈Sites | Files | Silent failure on drift |
|---|---|---:|---:|---|
| Kind compares | `.kind()==`, `!=`, `match .kind(){}`, `matches!(.kind(),..)`, cached `let k=node.kind()` | **150+** | ~20 | renamed kind → arm never fires |
| Field access | `child_by_field_name("x")` | **158** | 21 | renamed field → silent `None` |
| Navigation | `.parent()`, `.prev_sibling()`, `.next_sibling()`, `named_child(i)` | **58+** | 23 | inserted wrapper → off-by-one level |
| Queries | **6** S-expr consts in `language.rs` (`DEFINITIONS`, `CALLS`, `EVENT_SUBSCRIBERS`, `EVENT_PUBLISHERS`, `ATTRIBUTED_PROCEDURES`, `VARIABLES`) | 6 | 1 | compiles, matches **0** at runtime |

None fail loudly today. **The fix is not safer access to the CST — it is to stop the engine
seeing the CST at all.**

---

## 2. Goals / Non-goals

### Goals
- **G1 — One-way grammar boundary.** Exactly one layer (`src/syntax/`) knows tree-sitter. The
  engine (`src/engine/`, `parser.rs`/`handlers.rs` LSP front) consumes only the owned IR and
  never imports `tree_sitter`.
- **G2 — Grammar drift is LOUD.** A node/field the pinned grammar no longer contains is a
  **codegen/compile** failure; an unknown kind at runtime **panics**; a newly-appeared kind that
  nobody classified is a **failing coverage test**. No silent degradation to empty results.
- **G3 — Wrapper-insertion resilience.** A v3→v4-class wrapper insertion changes the **lowerer
  only**; zero engine changes. (The property the v2 hybrid and a public typed-CST both fail.)
- **G4 — Behaviour preservation, proven by dual-run.** During migration the IR path and the
  legacy CST path produce identical engine output on the corpus. Final outputs/goldens unchanged
  unless deliberately rebaselined.
- **G5 — Every IR node is traceable.** Each carries an `Origin` (raw kind string + tree-sitter
  id + byte range + positions) so anchors stay byte-identical (incl. `syntax_kind`) and findings
  point back to source.

### Non-goals
- **NG1 — Not the resolver redesign.** The `ReceiverType` work
  (`2026-06-13-call-graph-resolution-redesign.md`) is orthogonal; the boundary is **normalized
  syntax**, NOT a resolver HIR/SSA. Conflating them is gold-plating (§9).
- **NG2 — Not a lossless editor CST.** No rowan/cstree green-tree mirror of tree-sitter's shape
  (it just duplicates the coupling). We project to *semantic syntax*, not a faithful CST copy.
- **NG3 — Not incremental-recompute infra (yet).** No salsa. Lower once per parse, reuse across
  L2/L3/LSP. Revisit only if profiling demands it.
- **NG4 — No output-shape change as a side effect.** Any golden movement is deliberate and
  reviewed, never incidental to lowering.

> **This explicitly drops v2's `NG1: no second owned tree`.** Under the mantra, owning the IR —
> a second tree the engine reasons over — is the point.

---

## 3. Architecture

### 3.0 The pipeline

```
AL source
  │  tree-sitter parse (language.rs FFI — unchanged)
  ▼
tree_sitter::Tree                         ─┐
  │  GENERATED typed raw layer (private)   │  the ONLY
  ▼                                        │  grammar-aware
RawProcedure<'t> / RawCodeBlock<'t> / …    │  code lives
  │  HAND-WRITTEN lowerer + semantic schema │  in src/syntax/
  ▼                                        ─┘
owned AL IR  (AlFile → ObjectDecl → RoutineDecl → Block → Stmt → Expr, each + Origin)
  │
  ▼
L2 / L3 / L4 / L5 / LSP   ← never import tree_sitter
```

Three layers in the `al-syntax` crate: **generated raw facts** (compile-checked vocabulary + typed
CST wrappers), a **hand-written lowerer**, and the **owned IR**. The engine crate sits above the IR.

### 3.1 Workspace / crate layout

**The repo becomes a cargo workspace** (decided 2026-06-27) so the grammar boundary is enforced by
the **dependency graph, not a scanner**: only `al-syntax` depends on `tree-sitter`; at Phase 5
`al-engine` drops the dep, making `use tree_sitter` in engine code a permanent **compile error**.

```
Cargo.toml                     # [workspace] members = crates/*, xtask
vendor/tree-sitter-al/         # PINNED grammar (rev-locked, §5 Phase -1)
xtask/                         # gen-syntax codegen (reads node-types.json → al-syntax/.../generated/*)
crates/
  al-syntax/                   # the ONLY crate that depends on tree-sitter
    Cargo.toml                 #   tree-sitter, streaming-iterator, phf, string-interner
    build.rs                   #   FFI + GRAMMAR_NODE_TYPES_HASH assert (§3.2)
    src/
      lib.rs
      language.rs              #   tree-sitter FFI binding ONLY; queries gone (§3.7)
      parse.rs                 #   source → ParsedFile { ir: AlFile, issues, parse_status }
      raw/
        node.rs                #   pub(crate) RawNode<'t> lens (never leaves the crate)
        generated/             #   100% autogenerated, checked in, hash-guarded, never hand-edited
          kind.rs field.rs nodes.rs shape.rs
      schema/kind_policy.rs    #   EVERY RawKind classified into one Class (coverage-gated, §3.3)
      lower/                   #   the ONLY grammar-aware logic
        mod.rs error_policy.rs lower_object.rs lower_routine.rs lower_block.rs
        lower_stmt.rs lower_expr.rs lower_attributes.rs
      ir/                      #   the public surface: AlFile→ObjectDecl→RoutineDecl→Block→Stmt→Expr
        mod.rs origin.rs ids.rs file.rs object.rs routine.rs block.rs stmt.rs expr.rs
        attributes.rs types.rs
    tests/{fixtures, lowering_snapshots, recovery, grammar_shape}/
  al-engine/                   # L2-L5, graph, resolver. Depends on al-syntax. NO tree-sitter (post-P5)
    src/{l2, l3, l4, l5, graph, resolver, …}   # consumes al_syntax::ir only
  al-cli/                      # main.rs, server, handlers, bins (aldump/alsem). Depends on al-engine
```

During migration `al-engine` keeps a temporary `tree-sitter` dep (the legacy CST walk runs beside
the IR path for dual-run); Phase 5 removes it and the boundary turns permanent.

### 3.2 Generated raw layer (loud vocabulary + typed CST)

`xtask gen-syntax` reads `vendor/tree-sitter-al/src/node-types.json` (630 KB, verified present)
and emits:

```rust
// generated/kind.rs  (DO NOT EDIT)
pub enum RawKind { Procedure, CodeBlock, StatementBlock, DeclarationBody, /* …all… */ Error, Missing }
impl RawKind {
    pub fn from_raw(s: &str) -> RawKind {                 // LOUD: unknown == binary/grammar mismatch
        RAW_KIND_BY_STR.get(s).copied()
            .unwrap_or_else(|| panic!("unknown node kind {s:?} — grammar/binary mismatch"))
    }
}
pub const GRAMMAR_NODE_TYPES_HASH: u64 = 0x…;            // vs node-types.json at build → §5 Phase -1
```

```rust
// generated/nodes.rs  — typed CST wrappers (Gemini's Tier 1, kept PRIVATE to syntax::)
pub struct RawCodeBlock<'t>(RawNode<'t>);
impl<'t> RawCodeBlock<'t> {
    // generated from node-types.json field facts: body is the v3 statement_block
    pub fn body(self) -> Option<RawStatementBlock<'t>> { /* typed */ }
}
```

The lowerer is written against these typed wrappers — so even grammar-aware code gets
compile-time safety (no `child_by_field_name("…")` typos, no stringly navigation). If v4 removes
`RawCodeBlock::body` or changes its type, **the lowerer fails `cargo check`** — Gemini's
compile-time-loudness property, confined to one layer.

### 3.3 Exhaustive kind classification (closes the silent-skip hole)

Gemini's objection to an owned IR: "the lowerer can silently skip nodes on drift." Closed by a
single **exhaustive `RawKind → Class` match** that a generated test gates for completeness:

```rust
// schema/kind_policy.rs — a generated test asserts this match is EXHAUSTIVE over RawKind
fn class_of(raw: RawKind) -> Class {
    match raw {
        RawKind::Procedure | RawKind::TriggerDeclaration   => Class::Semantic,
        RawKind::StatementBlock | RawKind::DeclarationBody
            | RawKind::VarBody | RawKind::CaseBody | RawKind::ReportBody => Class::Transparent,
        RawKind::Comment | RawKind::Pragma                 => Class::Trivia,
        RawKind::Error | RawKind::Missing                  => Class::Recovery,
        RawKind::WithKeyword | RawKind::DoKeyword           => Class::Ignored("punctuation"),
        // … a NEW grammar kind with no arm → coverage test FAILS → forced explicit decision
    }
}
```

A new/renamed kind cannot slip through: either `RawKind` lost a variant (compile error) or gained
one with no policy arm (coverage-test failure). Strictly stronger than v2's `Other(RawKind)` —
"known but unclassified" is now impossible.

**No declarative `semantic_slots!` DSL** (dropped per Gemini's round-2 review as redundant
two-sources-of-truth gold-plating). Wrapper descent is done **explicitly in the lowerer via the
typed Tier-1 wrappers** — `RawCodeBlock::body()` already *is* the statement-block descent (§3.2),
so a parallel declarative "`StatementBlock` is transparent" rule would just restate the generated
field facts. The exhaustive `class_of` match stays (the loudness gate for new kinds); descent
logic lives in `lower_*.rs`, not a macro.

### 3.4 The lowerer (only grammar-aware code)

Hand-written, one module per IR level. It owns the analyzer semantics node-types.json can't
express: what is a statement vs trivia, call/assignment/member decomposition, attribute-to-
procedure association, with-receiver, condition extraction, v2/v3 alias decisions. It descends
transparent wrappers via the schema, drops trivia, applies recovery policy, and stamps `Origin`.
Unknown-but-classified-semantic nodes lower to an explicit `Stmt::Unknown { origin, issue }` /
`Expr::Unknown { … }` — **never silent omission** (preserves the failure as data).

### 3.5 The owned AL IR

```rust
// ir/origin.rs
pub struct Origin {                 // every IR node carries one
    pub raw_kind: RawKind,
    pub raw_kind_text: &'static str,// → anchor syntax_kind, byte-identical (G5, §3.8)
    pub ts_id: usize,               // node.id() parity (op/callsite maps key on this today)
    pub byte_range: Range<usize>,
    pub start: Point, pub end: Point,
}
// ir/stmt.rs (sketch — taxonomy mirrors what body_walk already distinguishes)
pub enum Stmt {
    Assignment { target: ExprId, value: ExprId, op_origin: Origin },
    Call(ExprId), If { cond: ExprId, then_: BlockId, else_: Option<BlockId> },
    Case { scrutinee: ExprId, branches: Vec<CaseBranch>, else_: Option<BlockId> },
    Repeat { body: BlockId, until: ExprId }, While { cond: ExprId, body: BlockId },
    For { /* … */ }, Foreach { /* … */ }, With { receiver: ExprId, body: BlockId },
    Try { body: BlockId, catch_: Option<BlockId> },   // → has_branching (body_walk.rs:819)
    AssertError(BlockId),                             // sets under_asserterror context (body_walk.rs:200)
    Exit(Option<ExprId>), Break, Unknown { origin: Origin, issue: SyntaxIssue },
}
// NOTE: taxonomy MUST cover every legacy feature-producing kind. Verified present in body_walk
// today and easy to drop: `try_statement` (hasBranching), `asserterror_statement`
// (under_asserterror — a CONTEXT that flows to descendant callsites, not just a node),
// `foreach_statement`, and `case_else_branch` (carried as Case.else_). A lowering fixture is
// REQUIRED for each (§8).
pub enum Expr {
    Identifier(Symbol), QuotedIdentifier(Symbol),
    Member { object: ExprId, member: Symbol },
    Call { function: ExprId, args: Vec<ExprId> },
    Literal(Literal), Unary { op, operand: ExprId }, Binary { op, lhs: ExprId, rhs: ExprId },
    Parenthesized(ExprId), QualifiedEnumValue { /* … */ }, DatabaseReference { /* … */ },
    Unknown { origin: Origin, issue: SyntaxIssue },
}
```

**Arena-backed** (`la-arena` or local `Vec`+typed ids: `ExprId`/`BlockId`/`StmtId`) — zero
per-node heap churn, cache-friendly, cheap to pass by id. Spans index into source; names interned
via the existing `string-interner` only where it already pays off. Lowered once per parse.

### 3.6 ERROR / MISSING (recovery)

LSP edits live, half-typed files → tree-sitter emits `ERROR`/`MISSING`. Centralized in the
lowerer (not scattered at access sites):
- Production: skip recovery nodes in semantic vectors where safe; record a `SyntaxIssue` with
  range + raw kind + context; set `ParseStatus` so a recovery-affected analyzer-critical region
  is not cached as authoritative.
- Strict (`cfg(test)` / flag): an unexpected `ERROR` under a construct the lowerer claims to
  handle is a **test failure** — the corpus surfaces recovery gaps instead of hiding them.

### 3.7 The 6 queries retire

With the IR, the `language.rs` S-expr queries stop being analyzer primitives:
- definitions → `AlFile.objects[*].members`; calls → IR traversal over `Stmt`/`Expr`; variables →
  `RoutineDecl.locals` + object globals + table fields; subscribers/publishers → normalized
  `attributes: Vec<Attribute>` on routines.
- Any query that must remain for parser-layer reasons stays *inside* `src/syntax/raw/` with
  per-query fixture tests (snippet → expected captures; compile-validation alone is insufficient
  — a wrapper insertion makes a valid query match zero). End-state target: **zero** queries in the
  engine.

### 3.8 Output invariance — anchors, ids, AND ORDER

`Origin.raw_kind_text` feeds `PAnchor.syntax_kind` verbatim (today `node.kind().to_string()`,
`body_walk.rs:98`). Columns keep the existing byte-column identity pass-through (`Utf16Cols`).
But byte-identical *anchors* are not enough — L2 is **order- and identity-sensitive**, and the IR
must preserve those properties or the engine output diverges. Three invariants the lowerer MUST
honour (raised in round-2 review, verified against `body_walk.rs`):

**INV-1 — Document order, wrappers spliced in place.** IR child vectors are stored in tree-sitter
**named-child document order**, with transparent wrappers (statement_block, …) flattened *at their
original position* (exactly today's `block_statements`, `node_util.rs:30`). Order-sensitive
streams — op/callsite numbering, loop ids/loopStack, the CFN statement tree, `fieldAccesses`/
`callSites`/`operationSites` (emitted in visit order), `identifierReferences` — **must NOT be
re-sorted by byte range.** Reason: nested `call_expression`s **share a start position**
(`body_walk.rs:4-5`), so byte-start ordering is ambiguous; only traversal order is well-defined.

**INV-2 — Emitted-feature Origin = the exact raw node legacy used.** For every op/callsite/
field-access/condition-reference the IR path emits, its anchor and event key come from the *same*
raw CST node the legacy extractor used: `call_expression` callsite → the `call_expression` node;
a parenless member call in statement position → the `member_expression` node; assignment → the
`assignment_statement` node. Derived IR containers (`Block`, `CaseBranch`) that emit no feature
need no faithful `ts_id`.

**INV-3 — Visit order within a construct is preserved.** Notably `with_statement` visits the
**receiver before the body** with an implicit-receiver frame pushed between (`body_walk.rs:954`
then `:969`→`:971`); a lowering that reorders these (or that records `under_asserterror`/
`implicit_receiver` after descending) breaks parity. The lowerer reproduces the legacy visit
sequence for every compound statement.

**`ts_id` is STRICTLY EPHEMERAL.** `Origin.ts_id` mirrors `node.id()` solely to build the L2
two-phase op/callsite maps during a single lowering pass. It must **never** be stored in cached
L3+ structures or compared across parses — tree-sitter recycles node ids across incremental
re-parses, so a persisted `ts_id` is a latent corruption bug. (Round-2 review, Gemini.) Mark the
field `#[doc(hidden)]`-style internal and assert (debug) it is not serialized.

These invariants are *proven*, not assumed, by the tiered dual-run (§5/§8.3).

---

## 4. Acceptance criteria

- **A1:** No `src/engine/**`, `parser.rs`, `handlers.rs`, `snapshot.rs`, `analysis.rs`, `main.rs`
  imports `tree_sitter` or names a raw grammar string. `tree_sitter::Node`, `child_by_field_name`,
  `.kind()` string compares, and sibling/parent nav appear **only** under `src/syntax/raw/` and
  `src/syntax/lower/` (boundary scanner, §8.4).
- **A2:** Removing a grammar node/field used by the lowerer is a compile error; an unknown raw kind
  panics; a new unclassified `RawKind` fails the coverage test (§3.3).
- **A3:** A simulated v4 wrapper insertion re-greens with edits confined to `src/syntax/`
  (lowerer + schema) — zero `src/engine/**` edits. (G3.)
- **A4:** Dual-run parity: across the BC corpus, IR-path engine output == legacy-path output
  (feature streams, then full `analyze`), byte-for-byte, before the legacy path is deleted.
- **A5:** Full `cargo test` (al-sem differential + Rust-owned goldens + contracts + IR snapshots +
  lowerer fixtures + recovery fixtures + coverage) passes with zero golden edits at every phase.
- **A6:** `GRAMMAR_NODE_TYPES_HASH` matches the checked-in `node-types.json`; mismatch fails build.
- **A7:** No perf-target regression, measured (§7).

---

## 5. Migration plan — dual-run is the spine

Each phase: `cargo test` (zero divergences) + boundary scanner + perf check where noted. No phase
rebaselines a golden. `superpowers:subagent-driven-development`.

### Phase -2 — Isolate + characterize legacy (setup, lands first)
- [ ] **Worktree** off `master` on branch `feat/owned-syntax-ir` (decided 2026-06-27) — `master`
  stays clean for the active moat/resolution work; no half-migration risks a release.
- [ ] **Characterize legacy behaviour FIRST** (Feathers): freeze the CURRENT L2/L3 feature streams
  as goldens so dual-run (§5 Phase 1) has a fixed target that cannot move while we refactor. This
  is a *pre-port* safety net, separate from the al-sem differential goldens.

### Phase -1 — Pin the grammar + convert to a workspace (precondition)
- [ ] Vendor tree-sitter-al at an exact rev; CI stops floating `main`. Add `GRAMMAR_NODE_TYPES_HASH`
  build check. (The v0.9.0 unpinned-HEAD incident mandates this regardless of the rest.)
- [ ] **Convert to a cargo workspace** (§3.1): split into `crates/al-syntax`, `crates/al-engine`,
  `crates/al-cli` + `xtask`. Mechanical move; `cargo test` green, zero behaviour change. `al-engine`
  keeps a *temporary* `tree-sitter` dep until Phase 5. (Doing the split now makes the boundary a
  build fact from the start, not a retrofit.)

### Phase 0 — Generate raw layer + IR scaffolding (no engine change)
- [ ] `xtask gen-syntax` → `al-syntax/src/raw/generated/{kind,field,nodes,shape}.rs` + hash.
  `--check` mode for CI.
- [ ] Hand-write `schema/kind_policy.rs` (exhaustive `class_of` + coverage test), `ir/` types,
  `lower/` skeleton, `parse.rs`. Unit tests: `RawKind::from_raw` panic-on-unknown; coverage
  exhaustiveness; typed-node accessors on v3 fixtures.
- [ ] Boundary scanner in CI **now**, seeded with the full current-coupling allowlist + a
  **monotonic-decrease** gate (no new raw coupling may enter during migration).

### Phase 1 — Lowerer to parity (the heart; split into 1a–1d, each independently shippable)
The **dual-run harness MUST parse once and fork the SAME `tree_sitter::Tree`** into both paths —
separate parses do not guarantee `node.id()` parity (INV-2, §3.8). Shape:
`parse once → Tree → {legacy CST extraction, lower→IR→IR extraction} → diff canonicalized output`.
- [ ] **1a** — raw generated layer + lowerer + IR snapshot fixtures (canonicalized `Origin`),
  one per construct incl. rare syntax + recovery states.
- [ ] **1b** — shadow **L2 body** feature-stream parity: diff EACH of `loops`, `operationSites`,
  `recordOperations`, `callSites`, `fieldAccesses`, `unreachableStatements`, `hasBranching`,
  `identifierReferences`, `varAssignments`, `conditionReferences`, and the op/cs id maps.
- [ ] **1c** — shadow CFN / control-context / operation-order parity.
- [ ] **1d** — shadow L3 projection parity (object/member/dataitem/property/attribute/global-var).
- [ ] Drive 1b–1d across the corpus (CDO/DC + repo fixtures **+ a malformed/live-edit fixture set**
  — clean BC source does NOT exercise the recovery policy, §8.6) until zero diff. **Canonicalization
  must NOT set-sort order-sensitive streams** (INV-1). Full `analyze` parity is the end-to-end gate,
  not the only gate. Do not start Phase 2 until 1b–1d are green.

### Phase 2 — Cut L2 over to the IR
- [ ] Reimplement `body_walk`/`cfn`/`classify`/`scope`/`control_context`/`operation_order`/
  `l2_workspace` to consume `syntax::ir` instead of `tree_sitter::Node`. `cargo test` + dual-run +
  perf after each module. Shrink the scanner allowlist each batch.

### Phase 3 — Cut L3 over to the IR (`l3_workspace`).
### Phase 4 — Cut the LSP front-end over (`parser.rs`/`handlers.rs`/`snapshot.rs`/`analysis.rs`/`main.rs`); retire the 6 queries (§3.7).
### Phase 5 — Seal
- [ ] Delete the legacy CST walk + `node_util` semantic helpers + the dual-run harness (or keep it
  behind a dev flag as a regression oracle). Scanner allowlist empty (modulo the `syntax::raw`/
  `syntax::lower` whitelist). Add clippy `disallowed_methods` as a second guard. `rustfmt` touched
  files. Update `CHANGELOG.md` + the `CLAUDE.md` grammar section to name the IR boundary canonical.

**Ordering:** pin → generate → lower-to-parity (proven by dual-run) → cut L2 → L3 → front-end →
seal. The legacy path stays alive and authoritative until each layer's IR replacement is parity-
proven, so the engine is never broken mid-migration.

---

## 6. "Never silently wrong" — the loudness guarantees

| Drift | Caught by | When |
|---|---|---|
| Node kind removed | `RawKind` lost variant → lowerer compile error | `cargo check` |
| Field removed/renamed | `FieldName`/typed accessor lost → lowerer compile error | `cargo check` |
| Unknown kind at runtime | `RawKind::from_raw` panic | parse time (loud) |
| New kind nobody classified | `kind_policy` coverage test fails | `cargo test` |
| Wrapper inserted (v3-class) | typed `body()` type change in lowerer | `cargo check` + fixtures |
| Order/visit-order perturbed | INV-1/INV-3 + dual-run order-preserving diff | `cargo test` / corpus |
| `ts_id` persisted across parses | debug assert: not serialized (§3.8) | `cargo test` |
| Query matches zero | per-query fixture test | `cargo test` |
| Grammar swapped under build | `GRAMMAR_NODE_TYPES_HASH` mismatch | build |
| Lowerer subtly wrong | dual-run parity diff + IR snapshots | `cargo test` / corpus |
| node-types.json ≠ checked-in | `xtask gen-syntax --check` | CI |

---

## 7. Performance (measured, not asserted)

The IR adds one lowering pass + arena allocation per parse, amortized across all of L2/L3/L4/L5/
LSP (today each re-walks the CST and re-allocates `Vec`s in `named_children`). Net is plausibly
neutral-to-better, but **measured, not assumed.** Mitigations baked in: arena/typed-ids (no
per-node `Box`), spans not strings, intern selectively, lower once. Validation: the 100/1000-file
index targets + sub-ms query targets in `CLAUDE.md`, before/after Phase 2 and at the end. Blockers:
>5% initial-index regression or any sub-ms target slipping.

---

## 8. Testing

- **8.1 Differential goldens (necessary).** al-sem R0/R1a + Rust-owned goldens + contracts; zero
  divergences, zero golden edits per phase.
- **8.2 Not sufficient** — they miss rare constructs, live-edit/ERROR states, zero-match queries,
  shape-preserving bugs. Hence 8.3–8.7.
- **8.3 Dual-run parity** (§5 Phase 1+) — primary cutover gate. **Tiered comparison surface**, not
  just final JSON: (1) IR snapshots, (2) L2 feature streams, (3) L3 feature streams, (4) full
  `analyze`. Parse-once/fork-same-Tree (INV-2). Canonicalization preserves order for
  order-sensitive streams (INV-1) — never set-sort op/callsite/loop/identifierReference order.
- **8.4 Boundary scanner** (Phase 0, monotonic): all coupling forms in A1; whitelist `syntax::raw`/
  `syntax::lower`; clippy `disallowed_methods` second guard at Phase 5.
- **8.5 IR snapshot fixtures** per construct (+ rare syntax).
- **8.6 Lowerer + recovery fixtures** — MUST include **malformed / live-edit / fuzzed AL** so the
  `ERROR`/`MISSING` policy is proven against legacy `body_walk` behaviour (clean BC corpus can't).
- **8.7 Raw-kind coverage + query fixtures** (§3.3, §3.7). A lowering fixture is **required for
  every legacy feature-producing raw kind** (or an explicit, reasoned waiver) — incl.
  `try_statement`, `asserterror_statement`, `foreach_statement`, `case_else_branch`.

### Known residual blind spots (a lowerer bug here can pass dual-run AND goldens)
1. **Construct absent from corpus** → mitigated by per-construct fixtures + raw-kind coverage.
2. **Latent IR field not yet consumed** → mitigated by IR snapshots (not just feature parity).
3. **Bug that preserves legacy behaviour** → dual-run proves *parity*, not language truth; accepted
   for this migration (al-sem-retired baseline is Rust-owned anyway).
4. **A new optional field on an already-classified node** (e.g. v4 adds `catch` to `try`) → the node
   is classified, but the hand-lowerer may forget the field; caught only if the corpus/fixtures
   contain that syntax. Standard logic-bug class, not an architectural silent-skip. (Gemini.)

---

## 9. Risks & where "best" becomes gold-plating

| Risk | Likelihood | Mitigation |
|---|---|---|
| Lowerer = central source of truth, subtly wrong while type-checking | **high** | Dual-run parity (A4) is non-negotiable; IR snapshots; per-construct fixtures |
| Over-normalization loses info/anchor parity | med | Semantic *syntax* IR not HIR (NG1); `Origin` on every node; `Unknown` not omission |
| Perf regression from lowering pass | low–med | Arena + measured gates (§7) |
| Goldens less diagnostic (lowerer vs analyzer bug) | med | IR snapshots localize; dual-run isolates; legacy path kept as oracle until Phase 5 |
| Census/scanner misses a coupling form | med | Scanner covers all forms; allowlist seeded from real grep, shrinks to zero |
| Migration spans many phases; risk of stalling half-done | med | Legacy path authoritative throughout; each phase independently shippable |

**Gold-plating line (unanimous across both reviewers):** ❌ rowan/cstree lossless CST mirror
(duplicates coupling); ❌ resolver HIR/CFG/SSA as the *first* boundary (wrecks golden locality,
conflates with the `ReceiverType` work — NG1); ❌ salsa/incremental now (NG3); ❌ auto-inferring
semantics from node-types.json (impossible — "`statement_block` is noise, `parenthesized_expr`
sometimes isn't" is human knowledge); ❌ `type-sitter` as the *architecture* (evaluate for raw
wrapper codegen only — it lacks grammar-hash enforcement, strict-field policy, origin
preservation, no-leak guarantees).

---

## 9b. Development setup & principles (decided 2026-06-27)

**Location:** a **git worktree** off `master` on `feat/owned-syntax-ir` (not a separate repo —
dual-run needs both paths in one binary; not in-place — `master` must stay clean for the active
moat work). **Structure:** a **cargo workspace** (§3.1) so the boundary is compiler-enforced.

The principles that decide the outcome, ranked:

1. **Boundary by compiler, not discipline.** `al-engine` cannot `use tree_sitter` after Phase 5 —
   it's a Cargo dependency fact. The scanner (§8.4) is the during-migration guard; the crate graph
   is the permanent one.
2. **Characterize legacy FIRST, then port** (Phase -2). Freeze current feature streams as goldens
   so dual-run has a fixed target while the lowerer is built. Pin behaviour before changing it.
3. **Dual-run parity is law — never rebaseline to pass.** A divergence = lowerer bug → root-cause
   it. Zero golden edits during a behaviour-preserving migration (mantra + al-sem-retired: rebaseline
   ONLY for intentional correctness gains, of which this migration has none).
4. **TDD the lowerer** (`superpowers:test-driven-development`) — the highest-risk component and the
   new source of truth. Test-first, per construct.
5. **Small reversible batches; legacy authoritative throughout** (`subagent-driven-development`).
   One module per commit, each independently shippable; the engine is never broken mid-migration.
6. **Generated code: checked in, hash-guarded, never hand-edited.**
7. **Measure, don't assert** — perf gates on the real `CLAUDE.md` numbers (§7).
8. **One concern per change — keep it orthogonal to the resolver work** (NG1). No accuracy
   improvements smuggled into the migration; they pollute parity and hide lowerer bugs.
9. **No new coupling enters during migration** — monotonic scanner from Phase 0.
10. **Repo hygiene** (CLAUDE.md): `rustfmt` per file (never `cargo fmt`), stage intended paths only,
    CHANGELOG per change, never push `master`.

---

## 10. Decisions & what changed v2 → v3

- **Architecture:** v2 hybrid CST seam → **owned AL syntax IR** (mantra + review). Drops v2 `NG1`.
- **Location/structure (2026-06-27):** git worktree `feat/owned-syntax-ir` + cargo workspace
  (`al-syntax`/`al-engine`/`al-cli`); boundary enforced by the dependency graph.
- **Both reviewers escalated and split:** Gemini → full generated typed CST + facade; GPT-5.5 Pro →
  owned IR, engine never sees `tree_sitter::Node`. **v3 merges:** Gemini's typed CST becomes the
  lowerer's *private, compile-safe substrate*; GPT's IR is the *engine's API*. The typed-CST alone
  was rejected as the public boundary because it makes wrappers first-class engine types (a v4
  wrapper still churns engine code — fails G3).
- **Codegen:** bespoke `xtask` over `node-types.json` (verified present, 630 KB); `type-sitter`
  evaluated for the raw wrapper layer only.
- **Build pipeline:** `xtask` (checked-in generated files, `--check` in CI), not proc-macro
  (obscures output, hurts IDE) and not primary `build.rs` (hash-assert only).
- **Resolved open questions (from v2):** Vec-vs-iter → arena+ids moots it; owned-IR → yes (the
  whole spec); `Other(RawKind)` → replaced by exhaustive `kind_policy`; enforcement → Phase 0
  scanner + Phase 5 clippy; parser/handlers → in scope (Phase 4); grammar pin → Phase -1, first.
- **IDs:** arena-backed (`la-arena`/local), evaluated for incremental later (NG3).

---

## 11. Review synthesis (two rounds)

**Round 1 (v1→v2):** both flagged v1 *relocated* silent failure — `NodeKind::Other` swallowed
renamed kinds; the API still let callers write the flat-iteration bug; codegen-from-node-types was
missing. v2 adopted generated `RawKind`/`FieldName`, `pub(crate)` raw nav, ERROR/MISSING policy,
fixture tests, Phase-0 enforcement, grammar pin. Also corrected facts: census undercounted; 6
queries not 4.

**Round 2 (mantra → v3):** both escalated past the hybrid. Gemini: typed-CST-as-API with compile-
time wrapper-insertion safety, rejected owned-IR (lowerer can skip silently). GPT-5.5 Pro: owned
semantic-syntax IR, rejected typed-CST-as-API (leaks grammar topology), with an exhaustive
raw-kind-classification gate that *answers* Gemini's silent-skip objection. v3 takes GPT's IR
boundary + Gemini's typed substrate + the classification gate, and makes **dual-run parity** the
migration spine to protect the correctness gate. Both drew the same gold-plating line (§9).

---

## Appendix A — Raw-kind classification seed (authoritative source: node-types.json + coverage test)

Semantic: object/field/key decls, `procedure`, `trigger_declaration`, statements (`if`/`while`/
`repeat`/`case`/`with`/`assignment`/`exit`/`break`/call), expressions (`member_expression`,
`call_expression`, `unary`/`comparison`/`parenthesized`, identifiers, literals,
`qualified_enum_value`, `database_reference`), `parameter`, `property`, `attribute_item`,
`report_dataitem`, `var_section`. — Transparent: `statement_block`, `declaration_body`
(+legacy `object_body`), `var_body`, `case_body`, `report_body`. — Trivia: comments, pragma,
begin/end/var/with/do/else keywords. — Recovery: `ERROR`, `MISSING`.

The generated coverage test forces every actual `RawKind` from the pinned `node-types.json` into
exactly one class; this list is a seed, not the source of truth.
