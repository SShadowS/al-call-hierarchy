# Receiver-closure + arg-increment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

> Status: **v2.1** (round 2: both GO-WITH-CHANGES → closers folded; task bodies REWRITTEN inline per gpt's execution-risk
> finding. BOTH addenda sections BINDING).

## Round-2 closers (BINDING)

- **T1 base-member union (gemini CRITICAL):** the closed gate validates against the UNION of [the source/symbol-declared
  controladdin procedures] AND [the platform base members every usercontrol instance has] — ground the base list from
  MS Learn (e.g. control-addin interface base: `Update`-class members; verify the actual documented surface, cite per
  entry) so valid platform invocations don't false-decline. Events still excluded.
- **T1 tri-state resolution (gpt):** addin-type resolution outcomes: Resolved(decl) → the gated lookup;
  TruePlatformSurface (no source/symbol candidate exists AND the name is a known platform addin — WebPageViewer class)
  → open-accept; Ambiguous (≥2 candidate declarations) → Unknown; Degraded (index/dependency incomplete, parse_incomplete
  decl) → Unknown. Open-accept NEVER serves ambiguity or degradation.
- **T1 arity (gpt):** if the controladdin lowering captures parameter lists, gate on arity too; if it does NOT, either
  add param capture in this task or fail CLOSED for declared addins (name-only acceptance is not permitted silently —
  investigate first, document the choice).
- **T3 same-scope-only duplicate rule (gemini):** the named-return fail-closed duplicate check applies ONLY to
  same-scope collisions (return-binding vs param vs local — malformed AL); a return binding SHADOWING a global is
  VALID AL and resolves to the binding (standard precedence), never fail-closed. Fixture both.
- **T4 var_passable = FALSE for member-field args, hardcoded (gemini, correcting v2):** AL requires a VARIABLE for
  `var` arguments — `Rec.Amount` cannot bind a var param ("A variable is required"). No investigation; encode false;
  the overload fixture asserts a var-param candidate is ELIMINATED for a member-field arg (sound elimination, mirrors
  the literal rule).
- **T4 enum catalogs split (gpt):** enum-TYPE static surface and enum-VALUE instance surface are separate closed
  catalogs; every member compiler/MS-Learn-cited; no open-ended lists. `AsInteger` is VALUE-surface (on `X::Y` chains),
  not TYPE-surface.
- **T4 enum collision rule programmatic (gpt):** implemented as `same_normalized_name && object_kind != Enum` over the
  whole object index — never a hardcoded kind subset.
- The task bodies below are REWRITTEN to match both addenda (gpt: implementers follow checklists, not preambles).

## Round-1 review addenda (BINDING)

**T1 — closed-if-known ControlAddIn gating (both reviewers' C1; supersedes the open-policy inheritance):**
- **Fork the policy**: if the controladdin OBJECT is resolvable (source- or symbol-declared — e.g. `CDO.Editor` has a
  .al `controladdin` object declaring procedures/events), the member MUST match a declared **procedure** (name; arity
  if declared — verify what the controladdin lowering captures; EVENTS are not AL-callable and do not satisfy the
  lookup). The route stays builtin Catalog — the declaration is an existence gate, not a new evidence class. If the
  addin type CANNOT be resolved (platform addins — WebPageViewer), the existing open-accept fallback applies,
  documented as unresolvable-surface policy.
- **Apply the same gate to the EXISTING direct-var `ControlAddIn "Foo"` path** — the current open-accept for
  resolvable addins is itself a latent false-Catalog vector; fix it in the same task (fixture: direct-var typo'd
  method on a source-declared addin → Unknown/MemberNotFound).
- Negative fixture: `CurrPage.X.Typo()` on a source-declared usercontrol → Unknown (not Catalog).
- **SystemPart is OUT of the ControlAddIn arm entirely** (both C2): native platform components, not JS addins. The 37
  CDO sites are all usercontrols — zero metric impact. SystemPart receivers: default-decline (Unknown) with a dated
  note (a closed SystemPart catalog is future work if real sites appear). SystemPart negative fixture mandatory.
- M1: rename (not just flip) the corrected negative tests so old semantics don't linger; keep the `.Page`-fabrication
  negative as its own test.

**T2 — context-sensitive zero-arg lookup (both C3; supersedes "collapse to ONE table key"):**
- KEEP the `is_method` schema distinction. Implement lookup context-sensitivity at the inference boundary:
  a zero-arg `Call` node → method row only; a bare `Member` (in receiver/arg-inference context) → exact property row
  FIRST, then the zero-arg method row as fallback; if BOTH rows exist with conflicting return kinds → Unknown
  (fail-closed). AUDIT existing `is_method:false` rows for same-receiver/member collisions BEFORE flipping any test.
- **Never in assignment/setter contexts**: the normalization applies only where the member is READ (receiver chains,
  arg positions). An assignment-LHS `X.Content := Y` must not become a call edge — verify the inference paths can't
  see LHS positions, or gate explicitly; fixture.
- ErrorInfo.CustomDimensions (I2): investigate the Dictionary return-kind representability FIRST; add closed rows with
  provenance if representable; if NOT, revise T2/T5 metric expectations explicitly (~8 becomes ~12) — no open
  dictionary surface, no silent metric miss.

**T3 — proven precedence + defensive duplicates (both; supersedes "Step-2 sibling"):**
- **Prove AL's bare-identifier precedence inside a table method BEFORE inserting the arm** (local vs param vs
  named-return vs global vs routine[parens-optional] vs implicit-self field) — a compiler-fixture-backed order note in
  the report. Insert the implicit-self field lookup at the PROVEN layer (expected: after globals and after the
  routine-shadow check — i.e. fields LAST among value symbols), not generically "in Step 2".
- Named-return binding: synthesized only in own-routine scope; on a duplicate with any other scoped value symbol
  (malformed source) → fail-closed for that identifier (decline, don't override). Fixtures: no-name return, duplicate
  local/param, QUOTED return-binding name, no cross-routine leakage, used-before-assignment (fine — typed).
- The binding carries the parsed return-type text verbatim into the synthesized VarDecl so arg dispatch behaves
  identically to explicit locals (gemini minor).

**T4 — widened shadow sets + honest var_passable + token-true with-scan (C5/C6/I1):**
- Bare-enum-type-name receiver: exactly-one Enum AND zero same-normalized-name objects of ANY other kind (table/page/
  codeunit/report/query/interface) AND no local/param/named-return/global/field/routine shadow. `Enum::"Type"` statics:
  a CLOSED enum-static catalog (Ordinals/FromInteger/Names/AsInteger/... — cite the AL surface), never open-accept.
- Member-field arg `var_passable`: VERIFY the AL rule (record fields ARE generally var-passable) and encode the truth;
  add an overload fixture where a wrong var_passable would flip the pick (the ByRef-exact path exercised both ways).
- The with-scan: LEXER-TOKEN-based (comments, string literals — AL `''` escaping — AND quoted identifiers excluded);
  if tokenization is unavailable/uncertain at that layer → conservative (treat as maybe-with, decline), never
  one-signal; fixtures: `// with`, block-comment with, `'with'` string, `"with"` quoted identifier, real `with` block.

**Cross-task (I4/I5):**
- T3 states its unknown delta explicitly: ~27→~12. The final ~8 is CONDITIONAL on ErrorInfo representability + all 4
  enum flips; state both-ways.
- **Per-task L3 preflight site ledger** (blocks landing): for each affected site — prior fresh bucket, prior L3 result,
  new fresh result, source adjudication, classification (matches_l3 / fresh_extra_verified / l3_disagreement / wrong).
  Any `wrong` or unexplained L3 disagreement blocks the task regardless of aggregate ratchet improvement. Tenth resolution arc (master `e430fe7`, CDO primary real-`unknown` 0.43% /
> 77: CompoundReceiver 51, UntrackedReceiver 18, MemberNotFound 7 [real eCandidates absences], BuiltinPrecedenceCollision
> 1; `ambiguousResolved=13`; `genuine_wrong=0`). Two grounding reports (this session) enumerated ALL 69
> CompoundReceiver+UntrackedReceiver sites against source and ALL 13 ambiguous sites: **the 69 are 100% mechanical —
> zero genuinely-dynamic residue**. Categories: (C) `CurrPage.<UserControl>` 37; (E) named-return-binding receivers 11;
> (A) parens-optional zero-arg framework form 9; (B)+(H) framework-row/implicit-self-field gaps 8; (D)(F)(G) enum-shape
> receivers 4. Of the 13 ambiguous: member-field args flip 3 (+2 partial), named-return fixes 2, the with-comment
> false-positive blocks 1; Enum::Value/bare-call-result increments = ZERO CDO yield (deferred).

**Goal:** Close the mechanical receiver population (77 → ~8, where the residual is the 7 verified-real absences + 1
collision) and shrink `ambiguousResolved` 13 → ~7 — all fail-closed, `genuine_wrong=0`, zero false `Source`/`Catalog`.

**Tech Stack:** Rust. Cross-crate (al-syntax lowerer) in T3. No `engine::l3`/`l2` imports in `src/program/resolve`
(grep-guarded). FOREGROUND cargo. Full CDO harness per task (`CDO_WS="U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud"`
+ `ENFORCE_CDO_WS=1`, `--test-threads=1`) — never a subset. Clippy bar: `--all-targets` clean.

## Key facts (verified on `e430fe7`; the grounding reports are authoritative)

- **(C) 37 sites** `CurrPage.<control>.Method(...)` where the control is a `usercontrol` (source-declared CDO.Editor /
  CDO.PrintService — 30) or platform `WebPageViewer` (7). Decline point: Step 0 (`receiver.rs:560-577`) requires the
  `currpage.<part>.page` shape + `PageControlKind::Part`; UserControl/SystemPart fall to Step 6 → generic 2-hop decline.
  The machinery EXISTS: `FrameworkKind::ControlAddIn` + the accepted open policy (`member_catalog.rs:447-448` — every
  ControlAddIn method is a JS-side platform invocation → builtin Catalog, no closed member set) already applies to
  direct `var X: ControlAddIn "Foo"` receivers; `find_page_control` (`receiver.rs:2002`) already merges PageExtension
  base controls. Missing wire: a Step-0 sibling matching `currpage.<control>.<member>` for UserControl/SystemPart
  entries → `Framework(ControlAddIn)`. The negative test `infer_currpage_usercontrol_dot_page_stays_unknown_not_fabricated`
  (`receiver.rs:4903`) gets EXTENDED (the `.Page`-suffix fabrication stays declined) not deleted.
- **(A) 9 sites** `Response.Content.ReadAs(...)`-style: zero-arg framework members written WITHOUT parens (idiomatic AL)
  parse as `ExprKind::Member` (`is_method=false`) and miss the `is_method:true, arity:0` table keys
  (`framework_returns.rs:146-149`). The module doc (`framework_returns.rs:35-37`) claims "AL procedures ALWAYS require
  parens" — FALSE (the user's standing correction, third recurrence; see the al-parens memory) — and the negative test
  `framework_chain_wrong_form_property_instead_of_method_declines` (`receiver.rs:5413-5449`) enforces the wrong
  behavior. FIX SYSTEMICALLY: normalize at the AST-recursion boundary (a bare zero-arg Member and a zero-arg Call
  collapse to ONE table key — true for every zero-arg AL procedure, not per-entry rows); flip the negative test as a
  correctness rebaseline; correct the doc.
- **(B) 4 sites** `ErrorInfo.CustomDimensions.{ContainsKey,Get}` — missing `framework_returns` rows + possibly a
  Dictionary-like `FrameworkKind` (check; add honestly or decline-documented). **(H) 4 sites** implicit-self table
  fields (bare `Attachment.CreateInStream(...)` inside the table's own procedure) — Step 2 has no arm; mirror the
  existing `Rec."Field"` field-index machinery without the `Rec.` prefix (Table/TableExtension from_object only,
  routine-shadow-guarded per the parens memory).
- **(E) 11 sites + ambiguous #9/#10**: named-return-value bindings (`procedure X(): Name: Type`) — the NAME is discarded
  at lowering (`lower/mod.rs:691-693` drops `FieldName::ReturnValue`; `RoutineDecl.return_type` is text-only). New
  primitive: capture the binding name on `RoutineDecl`; synthesize a scoped `VarDecl{name, ty}` into Step 2's local scan
  (own-routine scope only; AL forbids shadowing the return name) AND the arg-typing caller-scope lookup. Cross-crate.
- **(D)(F)(G) 4 sites**: enum-shape receivers — `X::Y.AsInteger()` (enum-value-literal chain), `Enum::"Type".Ordinals()`
  (static enum-type call), `"Type".FromInteger(...)` (bare enum-type-name receiver). `infer_receiver_type_for_expr`'s
  match (`receiver.rs:993-1064`) lacks arms (falls to `_ => Unknown`). All static closed-form constructs. Needs: the
  QualifiedEnum/enum-type arms + an enum-type-name cross-check for the bare case (fail-closed: only when the token
  resolves to exactly one Enum object AND no var/field/routine shadows it).
- **Member-field ARG typing (3 clean ambiguous flips + 2 partial)**: `Foo(Rec.Field)` / `Foo(X."Field Name")` — reuse
  `field_in_table` + the caller-scope-exact base lookup (WithState-gated identically) + `unquote_identifier`; the
  result feeds the UNMODIFIED `pick_candidate` (all guards apply automatically; `var_passable:false` correct — a
  member-field expr is never var-passable... VERIFY: AL allows `var`-passing a field? A field ref CAN bind var params
  in AL — set var_passable honestly per AL semantics; if var-passable, say so and test the ByRef-exact path). Decline:
  implicit-Rec-without-declared-var bases, multi-hop chains.
- **Comment-aware with-token scan (restores ambiguous #5 + Step-3 declines)**: `routine_has_with_token`'s raw-text scan
  hits comment text (`UseContiniaAuthorization` — "with" in a comment → two-signal disagreement → Unknown → declines).
  Fix: scan token/lexeme stream excluding comments+string literals (the IR/lexer has this — find the right layer; if
  only raw text is available at that point, a comment-stripping pre-pass with exact fixture proof). The AST depth
  signal stays as the second signal (two-signal design unchanged — only the text signal gets comment-blind→comment-aware).
- **Deferred with ZERO CDO yield (do NOT build)**: Enum::Value ARG typing; bare-user-routine call-result args; builtin
  call-result args (needs a return-type catalog — future); member/cross-object call-result args (needs Step-6 chain
  reuse — future). The orphaned enum/callexpr fixture banks STAY as not-yet-flipped guards.
- **The `.dependencies/CDO` same-slug double-include** side-noted by grounding = the known deferred snapshot-root-cause
  roadmap item — OUT OF SCOPE, do not touch.

## Global Constraints

- `rustfmt <file>` per-file — NEVER `cargo fmt`. Stage only named files — NEVER `git add -A`. CHANGELOG per task.
  Gates: `cargo clippy --release --all-features --all-targets -- -D warnings` (the raised bar), `cargo fmt --check`,
  `cargo test --workspace`, the FULL CDO harness per task.
- **Soundness cardinal:** every new receiver arm is fail-closed (unresolvable/ambiguous/shadowed → Unknown, never
  guess); the ControlAddIn extension inherits the EXISTING open policy (documented rationale: JS-side surface, no
  closed AL-visible member set — extending from direct-var to CurrPage-control receivers is consistency); no new
  false-`Catalog` vector. Every CDO movement adjudicated per-site vs source; `genuine_wrong=0` hard; fresh/L3 bucket
  movements per the established acceptance discipline (these were L3-unresolved too mostly — VERIFY what L3 held
  per category BEFORE landing each task; matches-vs-fresh_extra adjudicated).
- **Correctness over compatibility (user directive):** the two wrong negative tests (parens-form, usercontrol-decline)
  flip as documented rebaselines; ratchets re-derived DOWN with dated notes per task.
- Determinism; additive-only export changes. Out of scope: ProvenAbsent machinery; the double-include root cause; the
  2 pinned grammar defects; ABI param retention; implicit conversions.

## Tasks

### Task 1: CurrPage UserControl receivers — closed-if-known gating (the 37)
**Files:** `receiver.rs` (the Step-0 sibling + tests), the controladdin decl surface (node_extract/index as needed),
fixtures; harness ratchets.
- [ ] Failing fixtures: `usercontrol(X; "My Addin")` (source-declared) + `CurrPage.X.Init(...)` where `Init` IS a
  declared procedure → Catalog; a declared-addin PLATFORM-BASE member call (per the base-member union) → Catalog;
  `CurrPage.X.Typo()` on the declared addin → Unknown (the closed gate); an EVENT name → Unknown (events not callable);
  platform `WebPageViewer` (no declaration anywhere) → open-accept Catalog; an AMBIGUOUS addin name (2 decls) →
  Unknown; a SYSTEMPART control → Unknown (default-decline, dated note); `CurrPage.X.Page.Foo()` fabrication → stays
  declined (renamed test); an unknown control name → Unknown; a PART control unchanged; the DIRECT-VAR path: a typo'd
  method on a source-declared `ControlAddIn "Foo"` var → Unknown (the same gate retrofitted).
- [ ] Implement: the tri-state resolution (Resolved→gated union lookup [declared procedures + platform base, arity per
  the closer]; TruePlatformSurface→open-accept; Ambiguous/Degraded→Unknown); the Step-0 sibling for UserControl ONLY;
  the direct-var retrofit; PageExtension base-control merge (`find_page_control` reuse).
- [ ] FULL CDO harness + the L3 preflight site ledger (all 37 sites; blocks on wrong/unexplained): expect
  CompoundReceiver 51→14, unknown 77→~40; ratchets re-derived, dated; `genuine_wrong=0`. Commit:
  `feat(resolve): CurrPage UserControl receivers — closed-if-known ControlAddIn gating (Task 1)`.

### Task 2: Parens-optional zero-arg normalization + framework rows (the 9+4)
**Files:** `receiver.rs` (the boundary normalization), `framework_returns.rs` (doc fix + ErrorInfo rows), tests.
- [ ] Failing fixtures: `Response.Content.ReadAs(X)` parens-less `Content` → resolves (today declines); explicit-parens
  form still resolves (one table key serves both); the flipped negative test (documented rebaseline citing the parens
  memory); `ErrInfo.CustomDimensions.ContainsKey(K)` → resolves (or decline-documented if the Dictionary kind is
  unrepresentable — investigate FIRST, add the kind honestly if needed); a genuinely-absent zero-arg member still
  declines (the miss-declines design holds).
- [ ] Implement: CONTEXT-SENSITIVE lookup keeping the `is_method` schema (zero-arg `Call` → method row; bare READ
  `Member` → property row first, then zero-arg method fallback; both-exist-with-conflicting-kinds → Unknown;
  assignment-LHS never a call — verify/gate + fixture); the pre-flip audit of existing `is_method:false` rows; correct
  the false doc; the ErrorInfo rows w/ provenance (representability investigated FIRST — if unrepresentable, the
  metric expectations below shift +4 explicitly).
- [ ] FULL CDO harness + the L3 preflight site ledger: expect CompoundReceiver 14→1 (the (D) enum chain remains for
  T4), unknown ~40→~27 (or ~31 if ErrorInfo unrepresentable); ratchets; `genuine_wrong=0`. Commit:
  `fix(resolve): zero-arg framework members resolve parens-less — parens are optional in AL (Task 2)`.

### Task 3: Named-return bindings + implicit-self fields (the 11+4, + ambiguous #9/#10)
**Files:** `crates/al-syntax/src/lower/mod.rs` (+ ir/decl.rs: the binding name), `receiver.rs` (Step-2 synthesize +
implicit-self arm), `arg_dispatch.rs` (the caller-scope lookup gains the binding), `node_extract.rs` if RoutineNode
needs it; cross-crate tests.
- [ ] Failing fixtures (lowering): `procedure X() Ret: Record Y` → the binding NAME captured on RoutineDecl (today
  discarded); no-name return (`: Integer` only) → None. (Receiver): `Ret.Get(...)` mid-body → resolves via the
  synthesized scoped var; the binding does NOT leak to other routines; a same-named LOCAL cannot coexist (AL forbids —
  parse-level; defensive precedence documented). (Implicit-self): bare `Attachment.CreateInStream(X)` inside the
  owning table's procedure → field-index chain resolves; routine-shadow guard (a same-named procedure wins — the
  parens memory); a non-Table object → unchanged. (Arg): `GetJsonAttribute(.., ReturnValue)` shape → the binding types
  the arg → flips (#9/#10-shaped fixture).
- [ ] Implement (lowerer first — run the FULL workspace suite incl. al-syntax + inspect any golden movement). The
  implicit-self field arm inserts at the PROVEN precedence layer (the compiler-fixture-backed order note lands BEFORE
  the arm; expected: fields last among value symbols, after globals + the routine-shadow check). The named-return
  duplicate rule is SAME-SCOPE-ONLY (vs param/local = malformed → fail-closed; shadowing a GLOBAL = valid AL → the
  binding wins; fixture both).
- [ ] FULL CDO harness + the L3 preflight site ledger: expect UntrackedReceiver 18→~3, unknown ~27→~12 (or ~31→~16),
  ambiguousResolved 13→11; ratchets; `genuine_wrong=0`. Commit:
  `feat(resolve): named-return-value bindings + implicit-self table fields — receiver and arg typing (Task 3)`.

### Task 4: Enum-shape receivers + member-field args + the comment-aware with scan
**Files:** `receiver.rs` (the enum arms + the bare-enum-type check), `arg_dispatch.rs` (member-field args),
`extract.rs` (the with-scan), tests.
- [ ] Failing fixtures: `Rec.Field::Value.AsInteger()` chain → resolves; `Enum::"Type".Ordinals()` → resolves (the
  static enum surface — what catalog backs Ordinals/FromInteger/Names? Verify the enum-statics catalog exists or add
  honestly); `"Type".FromInteger(...)` bare enum-type-name → resolves ONLY when exactly-one Enum matches and nothing
  shadows; shadow negative. (Args): `Foo(Rec.Field)` + `Foo(X."Quoted Field")` flip fixtures + implicit-Rec-base and
  multi-hop declines + the var-passability decision tested per real AL semantics. (With): the comment-"with" fixture →
  `NoWithProven` (restores typing); a REAL `with` block still gates; string-literal "with" excluded too.
- [ ] Implement all three per the closers: the SPLIT enum catalogs (TYPE-static vs VALUE-instance, every member cited,
  `AsInteger` = VALUE-surface); the programmatic collision rule (`same_normalized_name && kind != Enum` over the whole
  index); member-field args `var_passable:false` HARDCODED (AL: a variable is required for var — the overload fixture
  asserts a var-param candidate is eliminated); the with-scan LEXER-TOKEN-based (uncertain → conservative; two-signal
  retained).
- [ ] FULL CDO harness + the L3 preflight site ledger: expect CompoundReceiver→0, UntrackedReceiver→0, unknown → ~8
  (the 7 absences + 1 collision; +4 if ErrorInfo unrepresentable); ambiguousResolved 11→~7 (3 member-field flips + #5
  restored); ratchets; `genuine_wrong=0`. Commit:
  `feat(resolve): enum-shape receivers, member-field arg dispatch, comment-aware with scan (Task 4)`.

### Task 5: Measure + close
- [ ] Full re-measure; adjudication sign-off (every delta == the per-task sums); ratchets at floors, dated; CHANGELOG
  capstone (the 100%-mechanical-population story; the residual = 7 verified-real absences + 1 collision + the honest
  ambiguous ~7; DEFERRED visible: ProvenAbsent for the 7, builtin/member call-result args, ABI param retention, the
  2 grammar defects, the double-include root cause); charter memory + MEMORY.md. Commit:
  `docs(resolve): receiver-closure arc complete — real-unknown 0.43%→~0.04% (Task 5)`.

## Roadmap — beyond this plan
ProvenAbsent for the 7 (consult recoveredFiles per the invariant); builtin-return-type catalog (call-result args);
member/cross-object call-result args; ABI param retention (SymbolOnly dispatch); the 2 tree-sitter-al grammar fixes;
the .dependencies double-include root cause; implicit-conversion modeling; protected Variables[]; Sender param-TYPE.
