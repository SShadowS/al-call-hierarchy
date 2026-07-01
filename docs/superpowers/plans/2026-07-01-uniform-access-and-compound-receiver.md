# Uniform-access `resolve_in_object` + compound-receiver resolution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

> Status: **v2.1** (external review round 2 = both GO-WITH-CHANGES; round-1 criticals confirmed closed, round-2 findings folded:
> the overload-narrowing false-`Source` guard, the `Func()` local-variable-shadowing decline, Task-2 golden-neutrality
> mechanics, framework-table per-entry provenance, the AST-native recursion helper, compiler-validated `this.` scope, scoped
> protected-ABI claims). Verification agent confirmed the enabling primitives. The next plan after three merged resolution arcs (master `fd34b91`→`f7856cb`, CDO real-`unknown`
> 6.46%→1.97%, residual stratified). Two grounded threads from the data-driven roadmap:
> 1. **Soundness (opus-elevated):** make `resolve_in_object` UNIFORMLY access-aware — but as a PER-CANDIDATE filter, not an
>    existential "some visible candidate exists" check. Closes the `ReceiverType::Object` arm + both `Interface`-impl
>    delegates still emitting false `Source` for cross-app `internal`/other-object `local` members.
> 2. **Burndown (the compound half of the 73% lever):** resolve the `compoundReceiver` bucket (167 CDO edges, 47%) via two
>    FAIL-CLOSED single-hop receiver-type resolvers — `Func().Method()` (typed by REUSING the bare-call resolver, not a naive
>    scan) and `<Framework>.<Prop|Method()>` (a versioned static table) — operating on the PARSED receiver AST, not string
>    splitting. Enabled by a dedicated infra task (thread the receiver `ExprId`; add `return_type` to the source
>    `RoutineNode`). Node-model-heavy shapes (record-field, cross-object chains) + `untrackedReceiver` remain DEFERRED.

**Goal:** Close the last `resolve_in_object` access-filter gap (fail-closed — count may rise) AND burn down the
`compoundReceiver` bucket with two fail-closed AST-based resolvers, driving CDO real-`unknown` DOWN — **adding zero
false-`Source`/`Catalog` claims** (`genuine_wrong` stays 0), each new resolution proven by compiler-semantics fixtures +
EXHAUSTIVE hand-adjudication of the CDO delta (not a sample).

**Architecture:** Task 1 (soundness) hardens `resolve_in_object` first. Task 2 adds the two enabling primitives (parsed
receiver `ExprId` reaches `infer_receiver_type`; `return_type` on source `RoutineNode`) with ZERO resolution change
(golden-neutral). Tasks 3–4 add fail-closed AST-based Phase-A steps to `infer_receiver_type`, both delegating typing to
EXISTING fail-closed machinery (`resolve_bare` for the prefix call; `parsed_type_to_receiver` for return-type→receiver, which
is `from_object`-scoped, cross-app-ambiguity-safe, and interface-safe). Task 5 re-measures + adjudicates + documents. The
clean-room reference is L3's `receiver_type.rs` (port the APPROACH over the IR; never import `engine::l3`).

**Tech Stack:** Rust (edition 2024, toolchain 1.96.0). No new dependency. No `engine::l3`/`engine::l2` import in
`src/program/resolve` except `builtins.rs::global_builtins` (grep-guarded).

**Source of truth:** the grounding + primitive-verification reports (this session, file:line-verified) + master `f7856cb` +
the charter (§5 taxonomy, §6 no-false-certainty, §8 metric).

## Key facts grounding this plan (verified on `f7856cb`)

**Uniform-access (Task 1):**
- `resolve_in_object` (`resolver.rs:203-261`): takes `(obj_id, obj_tier, name_lc, arity, graph, index, body_map)` — NO
  caller identity; ZERO access filtering. Its arity filter returns a SELECTED candidate (`:245-248`; 1→`Some`, >1→
  `Unresolved(OverloadAmbiguous)`).
- 7 callers: **A** `resolve_in_table_scope` (`:568`, pre-gated); **B** `resolve_bare` Step-1 own-object (`:677`, self);
  **C** `resolve_bare` Step-2 extension-base (`:726`, pre-gated Task 1.5); **D** `resolve_member` `ReceiverType::Object`
  (`:1333`, **GAP**); **E** `SelfObject` (`:1361`, self); **F** `Interface` SymbolOnly-impl delegate (`:1408`, **GAP**);
  **G** `Interface` Source-impl delegate (`:1429`, **GAP**). All have `from_object` in scope (`resolve_member` param `:1167`).
- `object_has_visible_member_candidate` (`resolver.rs:351-379`) EXISTS but answers EXISTENTIALLY ("some visible candidate").
  Task 1 needs a PER-CANDIDATE predicate. The per-`Access` rule (Public→visible; Local→`obj==from`; Internal→same-app;
  Protected→self OR `index.object_extends`; miss→fail-closed) + `access_exclusion_reason` + `lookup_routine_access`
  (`:299-305`) + `object_extends` (`index.rs:532-564`) are all reusable at the candidate level.
- **ABI/SymbolOnly is public-by-construction** (verified): `abi_ingest.rs:263` drops `is_local`/`is_internal`; `:283`
  hardcodes `Access::Public`; test `local_and_internal_routines_skipped` (`:562-647`). So `SymbolOnly→visible` cannot mask a
  cross-app INTERNAL/LOCAL false edge. KNOWN GAP (roadmap): `protected` has NO ABI-schema field (`RawMethod`/`AbiRoutine`
  lack `is_protected`, `symbol_reference.rs:170-184`) → a dep `protected` member in `SymbolReference.json` would be
  mislabeled `Public`; low-risk, documented, not closed here.
- **MUST NOT filter:** `Codeunit.Run(arity≤1)` (`resolver.rs:1299-1330`) + `resolve_object_run` (`:896-965`) — entry-trigger
  dispatch, platform-invoked, always-invokable; they already BYPASS `resolve_in_object`. Event-subscriber edges are a
  separate edge kind (not member dispatch through `resolve_in_object`) — Task 1 pins this with a fixture.

**Primitives (Task 2), verified:**
- NO parsed receiver AST reaches the resolver today: `CalleeShape::Member{receiver_text: String, method}` (`extract.rs:29-33`);
  `receiver_text` = `src[obj.origin.byte]` (`extract.rs:269/318`) — the structured `obj = file.ir.expr(*object)` (a full
  `Expr`, e.g. `ExprKind::Call{function,args}` `expr.rs:24-27`) is discarded. `infer_receiver_type` takes `receiver_lc: &str`
  (`receiver.rs:421-428`). The `AlFile` is IN SCOPE in the `full.rs` obligation loop (`full.rs:548-555`) — so the `ExprId`
  can be threaded through `CalleeShape::Member` → `ObligationKind::CallSite` (`full.rs:228-232/559-578`) → `infer_receiver_type`.
- `resolve_bare` (`resolver.rs:667-675`) is PURE (returns `Vec<Route>`; edge emission is in `full.rs:566-589`), fail-closed,
  precedence own-object→extension-base(access-filtered)→implicit-Rec(gated)→builtin→Unknown. `Func()` (parens) never matches
  `infer_receiver_type` Steps 0–4 today → reaches Unknown (Step 5). A resolver must call `resolve_bare` on the paren-stripped
  bare name explicitly.
- `RoutineNode` (`node_extract.rs:94-124`) LACKS `return_type`; `RoutineDecl.return_type: Option<String>` EXISTS
  (`crates/al-syntax/src/ir/decl.rs:133-176`); source extraction at `node_extract.rs:279-307` doesn't copy it. ABI carries
  `AbiRoutine.return_type_text` (`symbol_reference.rs:61`) but `abi_ingest.rs:279-294` discards it.
- `parsed_type_to_receiver` (`receiver.rs:826-868`) threads `from_object`, routes Record/Object through the fail-closed
  `resolve_object_ref` (`index.rs:414`; `Ambiguous`/`OutOfClosure`→`None`, NO blind-pick — verified), and maps an Interface
  return type to `ReceiverType::Interface` (polymorphic, `:859`), never a concrete id.

**Compound receivers (Tasks 3–4):**
- `ReceiverType::Unknown` → `UntrackedReceiver` (`resolver.rs:1468`); `full.rs:355-371` relabels to `CompoundReceiver` when
  `receiver_text.contains('.')`. `infer_receiver_type` (`receiver.rs:421-548`) Steps 0–4 only match a BARE identifier.
- L3 reference (port APPROACH): `compound_call_result_receiver` (`src/engine/l3/receiver_type.rs:579-687`),
  `compound_framework_property_kind` (`:527-552` + table `src/engine/l3/member_builtins.rs:380-458`), `strip_this_prefix`
  (`:893-900`). CDO measured `compoundReceiver=167`; L3's analogous residual is ~an order of magnitude smaller.

**Metric gates:** `cdo_full_program_coverage_and_self_reported_metric` (primary `<= 0.021`, measured 1.97% / `unknown=356`);
`cdo_l3_semantic_audit_no_fresh_wrong` (`genuine_wrong == 0`, `FRESH_MISSING_CEILING = 10`, `FRESH_WRONG_CEILING = 149`);
`sum(unknownByReason) == unknown`. `CDO_WS="U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud"`.

## Global Constraints

- Rust edition 2024; toolchain 1.96.0. `rustfmt <file>` per-file — NEVER `cargo fmt`. Stage only named files — NEVER
  `git add -A`. Update `CHANGELOG.md` per task.
- CI gates: `cargo clippy --release --all-features -- -D warnings` (NO `--tests`), `cargo fmt --check`, `cargo test
  --workspace` (NO `CDO_WS`, green). New fixtures under `tests/r0-corpus/`.
- **Soundness is the cardinal rule.** Task 1 makes resolution MORE fail-closed (count MAY rise — a false-`Source`→honest-
  `Unknown` correction is correct). The compound resolvers (Tasks 3–4) MUST fail closed: type a receiver ONLY when a UNIQUE,
  statically-proven type exists; decline on ANY ambiguity, overload, cross-app duplicate, scalar/Variant/interface-as-concrete,
  shadowing, non-empty/parse-uncertain args, table-miss, unversioned entry. A wrongly-typed receiver → a false `Source` =
  worse than `Unknown`. When in doubt, `Unknown`.
- **Reuse the fail-closed machinery; do NOT hand-roll.** Prefix-call typing goes through `resolve_bare` (inherits precedence
  + ambiguity-declines). Return-type→receiver goes through `parsed_type_to_receiver` (inherits `from_object`-scope, cross-app
  fail-closed, interface-safe). Never a name-only object lookup.
- **Operate on the parsed AST, never string-split `receiver_text`.** After Task 2, resolvers inspect the `Expr` node
  (`ExprKind::Call{function,args}` / `Member{base,member}`) — exact arity from `args.len()`, structured base to recurse on,
  no `rsplit('.')` (which corrupts on a string-literal-with-dot like `JToken.SelectToken('a.b')`).
- **`genuine_wrong == 0` is a REGRESSION backstop, NOT a semantic proof.** Correctness of each new resolution is proven by
  compiler-semantics fixtures (positives + fail-closed negatives) AND EXHAUSTIVE hand-adjudication of the CDO delta (every
  newly-`Resolved`/`Catalog` compound edge + every framework-table entry — NOT a sample), since the L3 oracle may share an
  error.
- `fresh_missing`-rise is NOT an absolute veto for Task 1: adjudicate each delta — an inaccessible-member false-`Source`
  removed (that L3 also had) is a soundness CORRECTION (accept + update golden/ratchet); an unrelated valid call newly missed
  is a regression (fix).
- Determinism (charter §C8). No `engine::l3`/`engine::l2` import in `src/program/resolve` except `builtins.rs::global_builtins`.
- **Out of scope (next plan):** record-field member-of-member + cross-object return-type chains; `untrackedReceiver`;
  honest-taxonomy reclassification; the `protected`-ABI-schema gap; the reason-overwrite precision fix; the `full.rs`
  histogram dedup. Tasks 3–4 assert these DEFERRED shapes stay `Unknown` (explicit negatives).

## File / module structure

| File | Responsibility |
|------|----------------|
| `src/program/resolve/resolver.rs` (modify) | Task 1: per-candidate access filter in `resolve_in_object`; thread all 7 callers; fix D/F/G. Task 3: prefix-call type-query via `resolve_bare`. |
| `src/program/resolve/extract.rs` + `full.rs` (modify) | Task 2: carry the receiver `ExprId` on `CalleeShape::Member` + the CallSite obligation into `infer_receiver_type`. |
| `src/program/node_extract.rs` + `abi_ingest.rs` (modify) | Task 2: add `return_type` to source `RoutineNode` (from `RoutineDecl.return_type`); ABI path stays `None` (return-type-less ABI is a deferred concern). |
| `src/program/resolve/receiver.rs` (modify) | Tasks 3–4: AST-based Phase-A steps in `infer_receiver_type` (call-result, framework-property, this-strip). |
| `src/program/resolve/member_catalog.rs` or new (modify/create) | Task 4: the versioned framework property/method return-type table. |
| `tests/r0-corpus/**` + `tests/program_resolve_harness.rs` (create/modify) | Tasks 1–5. |
| `CHANGELOG.md` + charter memory (modify) | Task 5. |

---

### Task 1: Per-candidate access-aware `resolve_in_object` (Object + Interface member gaps)

**Files:** Modify `src/program/resolve/resolver.rs`; Test `tests/r0-corpus/ws-object-interface-visibility/` + no-CDO harness + CDO gate.

- [ ] **Step 1: Write failing + control fixtures (full matrix).** POSITIVES (must still resolve `Source`): cross-app
  `public` via Object receiver; same-app `internal` via Object receiver; direct-extension `protected`; `SelfObject`
  `this.LocalProc()`; bare own `LocalProc()`; interface→public-impl `Method()`; `Codeunit.Run()` with an OnRun; `Page.RunModal`;
  `Report.Run`; a local `[EventSubscriber]` still receives its event. NEGATIVES (must become honest `Unknown`, assert exact
  pre-fix false route): Object-arm cross-app `internal`; Object-arm same-app `local` cross-object; **mixed-access same-arity
  overload** (`public Foo(Integer)` + `internal Foo(Text)` called cross-app as `Foo(...)` — pre-filter had 2 same-arity
  candidates, so access-narrowing to the lone visible `public` one must NOT manufacture a resolution: assert **NO `Source`
  at all** → honest `Unknown`, since arg types aren't proven);
  non-extension `protected` (same-app + cross-app); wrong-kind-extension `protected`; interface→internal-impl in an
  AL-valid internal-interface scenario (gaps F/G); a user-defined member named `Run` cross-app `internal` (NOT Run-exempt);
  `Codeunit.Run` on a codeunit with NO OnRun (→ no synthesized `Source`).
- [ ] **Step 2: Run — fail** (the negatives false-`Source`).
- [ ] **Step 3: Implement — PER-CANDIDATE filter, WITHOUT overload-narrowing.** Add `from_object: &ObjectNodeId` to
  `resolve_in_object`. Add a `routine_candidate_is_visible(candidate_id, from_object, graph, index)` predicate (the per-`Access`
  rule at the CONCRETE candidate). Compute `pre_filter_count` = arity-matched candidates BEFORE visibility; then filter by
  visibility. Selection rule (the guard that prevents access-filtering from MANUFACTURING a false `Source`): if 0 visible →
  `Unknown` (`access_exclusion_reason`); if exactly 1 visible AND `pre_filter_count == 1` → that route; if exactly 1 visible
  BUT `pre_filter_count > 1` (access removed a same-arity sibling) AND arg types are not proven → `Unknown(OverloadAmbiguous)`
  — do NOT select the lone survivor (the pre-filter set was ambiguous; access can't resolve which overload the call meant);
  if >1 visible same-arity with unproven arg types → `Unknown(OverloadAmbiguous)`. `SymbolOnly` tier → visible
  (public-by-construction, verified). Thread `&from_object.id` at all 7 callers (A/C no-op-redundant, B/E self-no-op, D/F/G the
  fixes) so Task 3's `resolve_bare` type-query inherits this guard. Do NOT touch `Codeunit.Run`/`resolve_object_run`.
- [ ] **Step 4: Run — pass** (all incl. positives). Then (WITH `CDO_WS`, SINGLE tests) `cdo_l3_semantic_audit_no_fresh_wrong`
  (`genuine_wrong` stays 0) + `cdo_full_program_coverage_and_self_reported_metric`. Count MAY rise (Object/Interface
  false-`Source` → honest `Unknown`); ADJUDICATE every newly-`Unknown` site (genuinely inaccessible?) AND every `fresh_missing`
  delta (inaccessible-member removal = accept + update golden; unrelated valid miss = fix). Raise the count ratchet with a
  dated soundness justification if it trips.
- [ ] **Step 5: rustfmt + clippy + (no-CDO) test + commit** — `fix(resolve): per-candidate access filter in resolve_in_object —
  Object-arm + Interface-impl member calls respect Local/Internal/Protected (Run/ObjectRun/events exempt)`.

---

### Task 2: Enabling primitives — thread the receiver `ExprId`; add `return_type` to source `RoutineNode` (golden-neutral)

**Files:** Modify `src/program/resolve/extract.rs`, `full.rs`, `receiver.rs` (signature), `src/program/node_extract.rs`; Test
`tests/program_resolve_harness.rs` + goldens.

- [ ] **Step 1: Write the invariant tests** — (a) a test that `infer_receiver_type` receives, for a `Func().M()` call, a
  structured receiver `Expr` whose kind is `ExprKind::Call` with `args.len()` == the source arg count (assert via a new
  unit that constructs the obligation and inspects the threaded node); (b) a test that a source routine `procedure P():
  Codeunit X` has `return_type == Some("Codeunit X")` on its `RoutineNode` (and an ABI routine's `RoutineNode.return_type`
  is `None` — ABI return types are a deferred concern). (c) NEUTRALITY: the full `cargo test --workspace` + CDO goldens are
  BYTE-IDENTICAL after this task (no resolution behavior changes — pure carry + field populate).
- [ ] **Step 2: Run — fail** (the node/return_type aren't available yet).
- [ ] **Step 3: Implement — resolution-neutral by construction.** (i) add `receiver: Option<ExprId>` (plus whatever file
  handle is needed to deref it) to `CalleeShape::Member` + the `CallSite` obligation; populate it at `extract.rs:252/269/318`
  (the `obj` node already exists); thread it to `infer_receiver_type` alongside `receiver_lc` + the `AlFile` so a resolver can
  `file.ir.expr(id)`. Existing Steps 0–4 keep using `receiver_lc` (no behavior change). **NEUTRALITY MECHANICS (mandatory):**
  the new `receiver` field MUST NOT participate in any output-affecting derive — exclude it from `PartialEq`/`Eq`/`Hash`/`Ord`
  and from any obligation DEDUP key (a manual impl or `#[derive]`-skip; if the struct is serialized, `#[serde(skip)]` +
  `#[serde(default)]`). Likewise `RoutineNode.return_type` must be excluded from any serialized node/graph snapshot the goldens
  compare (skip it, or scope the neutrality gate to edge/metric goldens). The point: obligation identity, ordering, dedup,
  edge output, and the unknown histogram are byte-for-byte unchanged. (ii) add `return_type: Option<String>` to `RoutineNode`
  (`node_extract.rs:94-124`); copy `RoutineDecl.return_type` in source extraction (`:279-307`); ABI path sets `None`.
- [ ] **Step 4: Run — pass** + FULL `cargo test --workspace` green. NEUTRALITY INVARIANT (assert explicitly): after this task,
  obligation COUNT, edge list, route ordering, and the `unknownByReason` histogram are unchanged, and CDO goldens are
  byte-identical (regen NOT expected; if a golden moves, STOP and find why — this task must be resolution-neutral). Confirm the
  grep-guard + `sum == unknown` hold.
- [ ] **Step 5: rustfmt + clippy + commit** — `feat(resolve): thread parsed receiver ExprId to infer_receiver_type + add
  return_type to source RoutineNode (enabling infra, resolution-neutral)`.

---

### Task 3: Resolve `Func().Method()` compound receivers (prefix typed via `resolve_bare`, fail-closed)

**Files:** Modify `src/program/resolve/receiver.rs` (+ a small `resolve_bare` type-query entry in `resolver.rs` if needed);
Test `tests/r0-corpus/ws-compound-call-result/` + CDO gate.

- [ ] **Step 1: Write failing + fail-closed-negative fixtures** — POSITIVE: `GetCustomer().Name()` where `GetCustomer()` is
  a bare same-object `procedure GetCustomer(): Record Customer` (unique, arity-0) and `Name` is a Customer proc → resolves
  `Source`, exact id. NEGATIVES (→ `Unknown`): (b) overloaded prefix (`GetX()` + `GetX(Text)` returning different types),
  called with args → wrong-overload guard; (c) scalar return (`GetCount(): Integer`); (d) absent/arity-mismatch prefix;
  (e) prefix name that ALSO matches an implicit-`Rec` method or a builtin (Rec-shadowing — must decline unless `resolve_bare`
  uniquely binds it); (f) cross-app-ambiguous return type (`GetH(): Codeunit Helper` with two deps defining `Helper`);
  (g) interface return type (`GetIFoo(): Interface IFoo` → polymorphic via `ReceiverType::Interface`, not a concrete guess);
  (h) DEFERRED-shape guard: `Obj.Method().X()` (cross-object chain) and a prefix with a string-literal-dot arg → stay `Unknown`.
- [ ] **Step 2: Run — fail** ((a) is `Unknown`).
- [ ] **Step 3: Implement (AST-based)** — new Phase-A step BEFORE the Unknown fall-through: if the receiver `Expr` is
  `ExprKind::Call{function, args}` where `function` is a bare identifier (NOT dotted/member — decline otherwise):
  **LOCAL-SHADOWING GUARD FIRST** — `resolve_bare` resolves ROUTINE calls and does NOT see locals/params/globals, but in AL a
  local variable, parameter, or array named `function_lc` SHADOWS a same-named procedure (so `GetCustomer(1).Name()` where
  `GetCustomer` is a local array is a variable-index access, NOT a call). So before typing via `resolve_bare`, look up
  `function_lc` in the caller's lexical scope (params + locals + globals via `body_map`/the var tables); if ANY variable/param
  matches, **fail closed → `Unknown`** (this plan does not type variable-backed receivers). Only if no such shadowing symbol
  exists: call `resolve_bare(from_object, function_lc, args.len(), …)` in type-query mode (take the returned `Vec<Route>`;
  require EXACTLY ONE route to a `RouteTarget::Routine(id)` whose `graph.routines[id].return_type` is `Some`); feed that
  return-type string through `classify_type_text` → `parsed_type_to_receiver(…, from_object, …)` → the receiver type; member
  resolution proceeds. DECLINE (→ `Unknown`) on: function not a bare identifier; a shadowing local/param/global exists;
  `resolve_bare` returns ≠1 route / a non-`Routine` target / an ambiguous or builtin route; `return_type` `None` or
  scalar-primitive; `parsed_type_to_receiver` yields `None`/ambiguous (cross-app duplicate) — inherited fail-closed. Interface
  return → `ReceiverType::Interface` (inherited). This reuses the bare-call precedence (own-object > extension-base >
  implicit-Rec > builtin) + ambiguity-declines + the local-shadowing guard, closing the shadowing + overload + cross-app holes
  structurally. Add a fixture: a local var named identically to an own procedure → the `Func()` receiver stays `Unknown`.
- [ ] **Step 4: Run — pass** (all incl. negatives). CDO gate (WITH `CDO_WS`): record the `compoundReceiver` + real-`unknown`
  drop; `genuine_wrong` stays 0; ADJUDICATE EVERY newly-`Resolved` call-result site (open CDO source: confirm the prefix's
  return type + the target member — exhaustive, not a sample). Deterministic; `sum == unknown` holds.
- [ ] **Step 5: rustfmt + clippy + (no-CDO) test + commit** — `feat(resolve): resolve Func().Method() compound receivers via
  resolve_bare-typed prefix return type, fail-closed (compound-call-result)`.

---

### Task 4: Resolve `<Framework>.<Prop|Method()>` compound receivers (versioned table) + `this.<rest>` (AST-based)

**Files:** Modify `src/program/resolve/receiver.rs`, add the framework table (`member_catalog.rs` or new module); Test
`tests/r0-corpus/ws-compound-framework/` + CDO gate.

- [ ] **Step 1: Write failing + negative fixtures** — POSITIVES: `Response.GetContent().ReadAsText()` (`Response:
  HttpResponseMessage` → `GetContent()`→`HttpContent` table entry → `ReadAsText` resolves); `JToken.AsObject().Get(...)`
  (→`JsonObject`); `this.DialogWindow.Open()` (this-strip → `DialogWindow` global → member resolves). NEGATIVES (→ `Unknown`):
  base not a known framework type; prop/method not in the table (table-miss = fail-closed); wrong arity/form (a table
  method-entry invoked as a property or with wrong arity); a base whose recursion mis-types (assert decline); a same-named
  member on a non-framework type must NOT hit the table; DEFERRED record-field `Rec.BlobField.X()` stays `Unknown`.
- [ ] **Step 2: Run — fail.**
- [ ] **Step 3: Implement (AST-based)** — (i) a fresh-native table keyed by `(FrameworkKind, member_lc, is_method: bool,
  arity: usize)` → returned kind, ported+validated from L3 `member_builtins.rs:380-458`. VALIDATION IS PER-ENTRY, not a header
  comment: each entry carries a provenance note (the exact BC platform/runtime version its AL semantics were confirmed
  against); OMIT any entry that (a) is unvalidated/uncertain, or (b) has a same-`(kind,member,form,arity)` overload with a
  DIFFERENT return kind (can't disambiguate without arg typing → not tabled). Add a module-level supported-runtime pin; if the
  workspace's platform/application version is outside the validated range, treat affected entries as DISABLED (fail closed),
  not best-effort. (ii) a Phase-A step operating on the AST: if the receiver `Expr` is `ExprKind::Member{base, member}` (or a
  `Call` whose function is a `Member`), recurse via an **AST-NATIVE helper `infer_receiver_type_for_expr(base_expr_id, …)`**
  that derives the base's type from the IR node directly — NOT by re-parsing `receiver_text` or the member name string (a text
  recurse would mis-handle `Response.GetContent().ReadAsText()` and string-literal-dot cases). If the base resolves to
  `Framework{kind}` and `(kind, member_lc, is_method, arity)` is in the table → the receiver is the returned kind; else
  decline. Add an invariant asserting the node shape for `Response.GetContent().ReadAsText()` is
  `Call(function=Member(base=<Response>, member=GetContent), args=[])`. (iii) `this`-strip: if the base `Expr` is the
  `this`/self identifier, resolve the member against a `SelfObject` self-only scope that includes ONLY the symbol classes AL
  actually permits addressing via `this.` (compiler-check the `this.DialogWindow.Open()` positive before implementing;
  EXCLUDE locals/params). If that self-only scope can't be cleanly distinguished from a shadowing bare lookup, DEFER `this.`
  stripping entirely (leave as `Unknown`) rather than risk a false `Source` on invalid syntax. All fail-closed.
- [ ] **Step 4: Run — pass** (incl. negatives). CDO gate: record the further drop; `genuine_wrong` 0; ADJUDICATE every
  newly-resolved framework/this site + every table entry actually hit on CDO; deterministic; `sum == unknown` holds.
- [ ] **Step 5: rustfmt + clippy + (no-CDO) test + commit** — `feat(resolve): resolve framework property/method compound
  receivers via versioned table + self-scoped this.<rest>, fail-closed (compound-framework)`.

---

### Task 5: Re-measure, adjudicate, tighten ratchets, CHANGELOG + charter memory

**Files:** Modify `tests/program_resolve_harness.rs` (ratchets), `CHANGELOG.md`, charter memory; Test.

- [ ] **Step 1: Full re-measure** (WITH `CDO_WS` + `ENFORCE_CDO_WS=1`, SINGLE tests): FINAL rate + `unknown` count + the FULL
  `unknownByReason` (the `compoundReceiver` bucket should have DROPPED; net against any Task-1 soundness rise). `genuine_wrong=0`,
  `sum == unknown`, `fresh_missing`.
- [ ] **Step 2: Exhaustive-adjudication sign-off** — confirm every newly-`Resolved`/`Catalog` compound edge from Tasks 3–4 was
  hand-adjudicated against CDO source (record the count + that it equals the bucket drop; note any that were declined as
  correctly-`Unknown`). This is the semantic proof, independent of `genuine_wrong`. **Protected-ABI guard:** if any adjudicated
  new `Catalog`/`Source` edge depends on a dependency member that is genuinely `protected` (mislabeled `Public` by the ABI
  schema gap), that edge is UNSOUND — the resolver path must be made to decline it (or this plan blocks on that edge). Confirm
  none of the adjudicated edges hit that case.
- [ ] **Step 3: Tighten ratchets to the measured floor** (counts): lower the `unknown` count ceiling if net dropped; if
  Task 1's soundness rise net-exceeded the burndown, RAISE with justification (soundness > coverage). Keep `genuine_wrong == 0`
  + `FRESH_WRONG`/`FRESH_MISSING`. Never loosen to hide a regression.
- [ ] **Step 4: Run** — (no CDO) green incl. `sum == unknown` + grep-guard; (WITH `CDO_WS`) all gates green, deterministic.
- [ ] **Step 5: CHANGELOG (honest, SCOPED claims)** — Task 1 per-candidate uniform-access (Object + Interface; Run/ObjectRun/
  events exempt). Word the completeness claim SCOPED: "uniform access for source routines + ABI-visible public/internal/local
  semantics; `protected` ABI members remain deferred due to schema loss" — do NOT claim uniform access is complete for all ABI
  access. Task 2 primitives; Tasks 3–4 compound-receiver resolution (with the MEASURED before/after). State what stays honest
  (record-field + cross-object chains = deferred; `untrackedReceiver`; reclassification; protected-ABI). No `engine::l3` import.
- [ ] **Step 6: Charter memory** — append: per-candidate uniform-access done; `compoundReceiver` burned down via the two
  fail-closed AST resolvers; real-`unknown` X%; next: record-field + cross-object return-type (node-model), `untrackedReceiver`,
  reclassification, protected-ABI. Update `MEMORY.md` pointer. Commit: `docs(resolve): uniform-access + compound-receiver
  resolution complete — real-unknown 1.97%→X% (Task 5)`.

---

## Roadmap — beyond this plan

Record-field member-of-member (`Rec.BlobField.X()` — Table field-type index on `ObjectNode`); cross-object return-type chains
(`ObjVar.Method().X()` — un-discard ABI `return_type_text`, chain-type the base first); `untrackedReceiver` residual; the
honest-taxonomy reclassification (overloadAmbiguous/memberNotFound/receiverOutOfClosure → charter §5 sub-states, gated,
proven per-route); the `protected`-ABI-schema gap (`IsProtected` ingestion); the reason-overwrite precision fix; the `full.rs`
histogram dedup. North-star (charter §8): workspace real-`unknown` to its provably-dynamic residual, risk-weighted by centrality.
