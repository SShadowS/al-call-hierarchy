# Phase 4 design: port the LSP front-end off tree-sitter

## Context

This repo is a Rust LSP server for AL (Business Central) providing call-hierarchy +
a deeper analysis engine (L2–L5). We are migrating ALL tree-sitter consumption behind
a single owned-syntax IR crate (`al-syntax`):

- `al_syntax::parse(source) -> AlFile` parses ONCE (tree-sitter) and lowers to an owned
  typed IR. The IR is the only thing downstream code sees.
- IR shape: `AlFile -> objects: Vec<ObjectDecl>`; each `ObjectDecl` has
  `kind` (Codeunit/Table/Page/...), `id`, `name`, `routines`, `globals`,
  `properties`, `report_dataitems`, `extends_target`, `implements`,
  `page_controls`, `fields` (number/name/data_type/field_class/is_blob_like),
  `keys`, `origin`.
- `RoutineDecl`: kind (Procedure/Trigger), name, params (name/by_ref/ty), return_type,
  locals, attributes (lowercased names), `attributes_parsed` (name + raw text + lowered
  arg exprs), access_modifier, parse_incomplete, dataitem_source_table,
  enclosing_member, body: Option<BlockId>, origin.
- Body: `Block { items: Vec<BlockItem> }`, `BlockItem = Stmt(StmtId) | Preproc(group with
  both #if/#else branches)`. `StmtKind` = Assignment/Call(Expr)/If/Case/While/Repeat/
  For/Foreach/With/Try/AssertError/Exit/Break/Continue/Block/Unknown.
  `ExprKind` = Identifier/QuotedIdentifier/Member{object,member}/Call{function,args}/
  Index/Literal/Unary/Binary{op}/Parenthesized/QualifiedEnum/DatabaseReference/
  RangeExpr/Unknown.
- Every IR node carries an `Origin` (byte range + line/col + kind_text).

**Already done:** the deep analysis engine (L2–L5) and L3 workspace assembly are fully
IR-driven and tree-sitter-free. Validated by dual-run byte-gate + Rust-owned goldens.

## What remains (Phase 4): the ORIGINAL LSP call-hierarchy front-end

This is a SEPARATE pipeline from the engine. Flow:
`AlParser.parse_file(source) -> ParsedFile` (in `src/parser.rs`, via 6 tree-sitter
S-expr queries in `src/language.rs`) -> `src/indexer.rs` builds `CallGraph`
(`src/graph.rs`) -> `src/handlers.rs` answers prepareCallHierarchy / incomingCalls /
outgoingCalls from the pre-built graph (positions come from stored ranges, NOT a live
tree walk).

`ParsedFile` fields the front-end needs:
- object_type / object_name (last object in file wins, legacy behavior)
- definitions: Vec<{name, range, kind(Proc/Trigger), complexity, parameter_count}>
- calls: Vec<{object: Option<String>, method, range, containing_procedure}>
- variables: Vec<{name, type_name, type_kind, containing_procedure}>
- event_subscribers: Vec<{subscriber_name, range, publisher_object_type, publisher_object,
  publisher_event}> (parsed from the [EventSubscriber(...)] attribute args)
- event_publishers: Vec<{name, range, selection_range, kind(Integration/Business/Internal),
  is_local, signature}>  (signature = textual procedure header)
- implicitly_invoked: Vec<String> (procedures with framework attrs [Test]/[*Handler])

### Tree-sitter consumers to port (production only):

1. **`src/parser.rs`** (~960 lines non-test): the 6 queries + extraction. THE core.
2. **`src/analysis.rs`** `calculate_complexity(node)` / `count_decision_points`:
   cyclomatic complexity over the AST. Counts: if (+1, +1 if else), while/for/foreach/
   repeat (+1), case_branch (+1), logical and/or (+1). Called per definition by parser.rs.
3. **`src/indexer.rs`**: owns the AlParser thread-local; calls parse_file -> add_to_graph.
   Mechanical: swap to the IR-based parse.
4. **`src/handlers.rs`** `field_properties` / `action_properties`: niche custom LSP
   requests. Re-parse a file from URI, walk the tree to a field_declaration /
   action_declaration by name, return ALL its `property` name=value pairs (+ field id).
5. **`src/main.rs`** CLI `--no-lsp` path (lines ~123–376): its OWN tree-sitter walk
   producing per-procedure metrics for CLI output (proc name, params, complexity).

## Proposed approach

**Core (parser.rs + analysis.rs):** rewrite `AlParser::parse_file` to call
`al_syntax::parse(source)` and project `ParsedFile` directly from the IR:
- definitions <- objects.routines (range = routine.origin; parameter_count = params.len();
  complexity = new IR-based cyclomatic walk).
- calls <- walk every routine body; `ExprKind::Call { function }` where function is
  Identifier => simple call (method only); function is Member{object, member} =>
  object.method. Range = call expr origin. containing_procedure = routine name.
  (Mirrors the CALLS query: call_expression with identifier or member_expression.)
- variables <- routine.locals + object.globals (type_kind/type_name parsed from the raw
  ty string with existing `parse_type_specification`). containing_procedure = routine name
  (None for globals).
- event_subscribers / event_publishers / implicitly_invoked <- routine.attributes_parsed
  (already lowered: name + raw text + arg exprs). Subscriber args parsed from the existing
  arg projection. Publisher `signature` = source slice from routine.origin.start to
  body.origin.start (the IR carries both byte ranges), reusing the existing whitespace
  normalizer — no tree walk needed.
- complexity: port `count_decision_points` to a recursive IR walk over Block/Stmt/Expr
  (If: +1, +1 if else_block; While/Repeat/For/Foreach: +1; each CaseBranch: +1;
  Binary{And|Or}: +1). Descend BOTH preproc branches (matches the legacy raw-tree walk
  which sees #if and #else nodes).

**Gap — field/action properties (handlers.rs):** the IR carries field number/name/type/
class but NOT arbitrary per-field properties, and actions are not in the IR at all
(page_controls only models part/systempart/usercontrol). Options:
  - (A) Extend the IR: add `properties: Vec<(name,value)>` to FieldDecl + add an
    actions/controls model. Heavier; these are rarely-used auxiliary requests.
  - (B) Have `al-syntax` expose a thin "raw CST query" API (al-syntax already owns
    tree-sitter) so these niche lookups run there, keeping the ENGINE/LSP crate
    tree-sitter-free. The Phase-5 goal is only that NON-al-syntax crates stop linking
    tree-sitter.
  - (C) Keep these two handlers on a direct tree-sitter parse for now, port later.

  Leaning (B): preserves the architectural invariant (al-syntax is the sole tree-sitter
  owner) without bloating the IR for niche data. Want reviewer opinion.

**main.rs CLI walk:** re-express on the same IR projection (it needs proc name / params /
complexity — all in the IR). Low risk.

## Validation strategy

- parser.rs has unit tests (object types, variables, calls+containing proc, params,
  event sub/pub extraction, real BC files). Keep them green.
- Add a differential test: for a corpus sample, assert the IR-based ParsedFile equals the
  old query-based ParsedFile (definitions/calls/variables/events) — a temporary dual-run
  gate, like we used for the engine port. Then delete the queries + AlParser tree-sitter.
- Full `cargo test` (engine goldens unaffected — separate pipeline).

## Questions for reviewers

1. Is projecting `ParsedFile` directly from the IR sound, or is there a front-end
   behavior (e.g. error-recovery call sites, malformed files) that the raw queries catch
   but an IR projection would miss? The engine port hit 4 serde-skip blind-spot bugs;
   what's the analogous trap here?
2. Field/action properties: option A vs B vs C? Any reason NOT to make al-syntax expose a
   raw-CST query helper for niche structural lookups?
3. The `calls` extraction: the legacy CALLS query matches `call_expression` only. The IR
   also has `StmtKind::Call(Expr)` (parenless statement calls like `Commit;`). Should the
   IR-based front-end now ALSO capture parenless calls the old query missed (a correctness
   improvement), or exactly reproduce the old query's call set first (zero-diff), then
   improve? (Mantra: best solution, not zero-diff — but call-graph edges are load-bearing.)
4. Cursor resolution: prepareCallHierarchy resolves the symbol-at-position from the
   pre-built graph's stored ranges, not a live tree walk — so no positional node lookup is
   needed for call-hierarchy. Confirm there's no hidden tree-sitter dependency in the
   position->symbol path. Is a positional ExprId@Point lookup in al-syntax still worth
   building now for future LSP features (hover/definition), or YAGNI?
5. Any sequencing risk: should the dual-run differential gate land BEFORE the rewrite (so
   we can prove zero-diff), or is the rewrite+test in one step acceptable?

---

## REVIEWER CONSENSUS (Gemini 3.1 Pro + GPT-5.5) + refined plan

Both reviewers reviewed the doc + source. Strong convergence. Decisions:

- **Q2 field/action props → B+ semantic facade in `al-syntax`** returning OWNED domain
  structs (no `tree_sitter::Node` / Query / capture names leak). Optional **A-lite**:
  add `properties: Vec<(String,String)>` to `FieldDecl` (fields are already in the IR
  and FieldClass already derives from field props — cheap + coherent). Actions: facade
  only (a full action/control IR model is over-engineering for a niche request). NOT C.
- **Q3 calls → zero-diff first** (only `ExprKind::Call`), parenless `StmtKind::Call`
  statement calls as a separate fast-follow with its own diff report + fixtures.
- **Q4 → YAGNI** on general positional `ExprId@Point`. BUT add `RoutineDecl.name_origin`
  NOW — `ParsedEventPublisher.selection_range` is the proc NAME node range, unreproducible
  from the current IR (only `name: String` + routine `origin`). Concrete, not speculative.
- **Q5 → dual-path coexistence**, not big-bang: keep legacy `AlParser` (TS) alongside a
  new `parse_file_ir`; differential-test IR==legacy on a corpus; classify+resolve diffs;
  switch indexer; THEN delete queries + legacy fields. Shadow-diff is a TEST/dev feature,
  NOT an unconditional production `assert_eq!` (would panic the LSP / double-parse).

### Concrete traps to honor (the analogue to the engine's serde-skip blind spots)

1. **`parse_incomplete` / `Unknown` subtrees are black holes.** Legacy queries find
   `call_expression`/`variable_declaration` inside error-recovered trees; the IR may
   lower a malformed stmt as `StmtKind::Unknown` (payload-free) and DROP its calls/vars.
   Honor `RoutineDecl.parse_incomplete` (the engine port used the legacy walk for these).
   Decide per-routine: if `parse_incomplete`, accept the diff or fall back. Add malformed
   fixtures to the differential test.
2. **Recursive call walk, not just `StmtKind::Call`.** Legacy matches `call_expression`
   ANYWHERE (in conditions, args, assignment RHS, return exprs, case patterns, loop
   bounds). The IR walker must descend every Expr under every Stmt.
3. **Faithful call-object rendering.** Legacy captures the member object as `(_)` — any
   node text: `Rec`, `CurrPage.MyPart`, `Arr[i]`, `SomeFactory()`, `Rec."Blob"`. A
   legacy-faithful renderer over `ExprKind` is needed, not just `object: Identifier`.
4. **Signature slice stops at `var` OR `begin`.** `body.origin.start` points at `begin`;
   legacy `find_body_start` stops at the first `var` SECTION too. Use
   `min(first-local-var origin, body origin)` (locals carry origins) or reuse the textual
   `find_body_start` over the routine slice. Otherwise the signature wrongly includes the
   local var section.
5. **Positions are BYTE columns.** Legacy `node_range` uses tree-sitter `.column` (byte
   offset in line) as the LSP `character`. Match that for zero-diff (UTF-16 correctness is
   a separate, later improvement). Confirm `Origin`'s column convention.
6. **Attribute case.** Legacy `#eq?` is case-SENSITIVE ("EventSubscriber"); IR
   `attributes` are lowercased. Real BC always writes exact case so in practice no diff;
   use case-insensitive match (more correct) and note it.
7. **Subscriber arg parsing** differs (legacy comma-split vs IR arg projection — strings
   with commas). IR is more correct; classify the diff as intentional from a baseline.
8. **Multi-name vars / param count.** Legacy returns only the FIRST name of a `names`-list
   `variable_declaration`; IR may expand to multiple `VarDecl`. `parameter_count`: legacy
   counts `parameter` NODES; `params.len()` may differ for grouped params. Verify.
9. **Object "last wins" + kind coverage.** Iterate `AlFile.objects` in source order,
   assigning object_type/name repeatedly (last wins). IR has Profile/Entitlement the
   legacy query lacks, and legacy forces `preproc_split_declaration` → Codeunit — don't
   silently change these under zero-diff.
10. **`variables` = object globals + routine locals ONLY** (legacy query is
    `(variable_declaration)`), NOT engine-only pseudo-vars (report dataitems etc.).

### Implementation order

1. al-syntax: add `RoutineDecl.name_origin: Origin` (lowerer) + (A-lite) `FieldDecl.properties`.
2. Keep `AlParser` (TS) as `parse_file` legacy; add `AlParser::parse_file_ir` (or a free
   `fn parse_file_ir(source) -> ParsedFile`) projecting from `al_syntax::parse`.
3. Port complexity to an IR walk (`if`+1/+else+1; loops+1; each CaseBranch+1;
   Binary{And,Or}+1; descend both preproc branches).
4. Differential test (fixtures + corpus sample): IR ParsedFile == legacy ParsedFile for
   definitions/calls/variables/events/implicitly_invoked. Resolve to zero/accepted diffs.
5. Switch `indexer.rs` to the IR path. Port `handlers.rs` field/action props to the
   al-syntax facade. Port `main.rs` CLI walk to the IR projection.
6. Delete the 6 queries + `AlParser` TS fields. Phase 5: drop engine `tree_sitter` dep.
