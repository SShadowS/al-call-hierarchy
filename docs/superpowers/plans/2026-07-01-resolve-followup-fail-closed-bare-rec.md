# Follow-up — Fail-closed object resolution (I1) + bare implicit-`Rec` (resolve_bare Step 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

> Status: **v2.1** — round 2 (gemini-3.1-pro GO + gpt-5.5 GO-WITH-CHANGES→GO, both "proceed"). Narrow round-2
> tightenings folded in: (1) Task 1 COMMITS to hardening the base APIs (`resolve_object`/`object_by_number` return
> `None` on >1 visible dep match, own-app-shadow preserved) — no "OR migrate" openness; a legit pick-first caller is
> renamed `..._first_by_stable_id` + grep-guarded. (2) Task 3 builtin/intrinsic guard is PROBE-THEN-DECIDE (probe the
> implicit-table scope; ANY same-name+arity candidate → `Unknown`; else fall through to `Catalog`) — not skip-before-probe
> (which would emit a wrong `Catalog` on a real collision). (3) Task 3 `with`-guard is TRI-STATE (`NoWithProven`→run;
> `HasWith`/`Unknown`→skip; raw-token `with` scan as the fallback when the AST can't localize — false positives fine,
> false negatives fatal). (4) Task 2 filters cross-app `Internal`/`Local` procedures from the candidate set (App A can't
> see App B's `internal` methods). (5) Task 3 structurally matches ONLY {Table, Page, TableExtension, PageExtension} —
> Report/XMLport → `Unknown`. (6) + a PageExtension base-vs-SourceTable precedence fixture.
>
> v2 — rewritten after gpt-5.5 GO-WITH-CHANGES + gemini-3.1-pro GO-WITH-CHANGES (round 1). Both found
> the same latent false-`Source` vectors; v2 closes them at the root:
> 1. **I1 was only half-closed.** The pick-first disease is in the BASE functions (`graph::resolve_object`,
>    `index::object_by_number`), not the `resolve_table_id` wrapper. v2 audits EVERY caller of those base functions,
>    makes them ambiguity-aware (or routes semantic callers through `resolve_object_ref`), and adds a grep-guard —
>    deleting `resolve_table_id` + migrating 2 callers alone is NOT sufficient.
> 2. **Bare-Rec `with`-block trap.** `with RecVar do SomeProc()` binds `SomeProc` to `RecVar`, NOT the implicit `Rec`.
>    Step 3 must be `with`-aware: investigate the IR; fail-closed for any bare call inside (or, if the IR can't
>    localize, any routine containing) a `with` statement.
> 3. **Bare-Rec builtin/intrinsic precedence is UNPROVEN.** A bare call matching BOTH a table proc AND a builtin (or
>    a page-intrinsic like `Update`/`Close`) must FAIL CLOSED (`Unknown`), not assume table-proc-wins — until the real
>    AL precedence is compiler-verified.
> 4. **Caller A numeric-vs-name dispatch is lossy** — `Record 18` and `Record "18"` both normalize to `"18"`; preserve
>    the object-ref shape or fail-closed on numeric-looking normalized names.
> 5. **`resolve_in_table_scope` must be visibility-scoped** to the caller's dependency closure (a latent gap in
>    `resolve_member` too), else it resolves TableExtension methods from apps the caller doesn't depend on.
> 6. **The L3 `genuine_wrong==0` gate is a REGRESSION backstop, not a semantic proof** — the plan's correctness proof
>    is compiler-backed/hand-verified precedence FIXTURES, added for every new semantic rule.

**Goal:** Eliminate the last pick-first false-`Source` vector (I1, at the root) AND close the bare-call implicit-`Rec`
gap (`resolve_bare` Step 3) with airtight fail-closed guards — **adding zero false-`Source` claims**, with the actual
`fresh_missing`/rate delta MEASURED, never assumed (the "82" is unverified).

**Architecture:** I1 first (Task 1) — the root pick-first fix + its fail-closed `resolve_object_ref` is REUSED by Step 3.
Task 2 makes the `resolve_member` Record-arm scope search visibility-scoped and extracts it to a shared helper
(characterization-tested). Task 3 builds `resolve_bare` Step 3 via that helper with the `with`/builtin/intrinsic guards.
Task 4 re-measures + ratchets. Clean-room reference for Step 3 = L3 `call_resolver.rs` Fallback-1.5 — but L3 is a
BEHAVIOR reference, never a correctness ORACLE; new semantic rules get compiler-backed fixtures.

**Tech Stack:** Rust (edition 2024, toolchain 1.96.0). No new dependency. No `engine::l3`/`engine::l2` import in
`src/program/resolve` (grep-guarded by `resolve_module_has_no_stray_engine_l3_l2_imports`, beyond-1B.3b Task 8).

**Source of truth:** the two grounding reports (this session, file:line-verified) + master `7aaa778`. The `.superpowers/sdd/task-8-report.md`
"82/102" is UNVERIFIED (deleted throwaway diagnostic + coarse different-object heuristic + ONE hand-checked site) —
a signal, not a target.

## Key facts grounding this plan (all file:line verified on master `7aaa778`)

**I1 (Task 1) — the pick-first is in the BASE functions, not the wrapper:**
- `resolve_table_id` (`src/program/resolve/receiver.rs:794-810`) is a thin wrapper → `Option<ObjectNodeId>`; numeric →
  `ResolveIndex::object_by_number`, name → `ProgramGraph::resolve_object`. Its 2 callers: `parsed_type_to_receiver`
  (`:762-772`, declared-var Record — has `from_app` not `ObjectNodeId`; `from_object` IS in scope at the `:497`/`:399`
  call site) + `infer_implicit_rec` TableExtension arm (`:544-551`, has `from_object`).
- **The actual pick-first lives in:** `ProgramGraph::resolve_object` (`src/program/graph.rs:52-88`, own-app-shortcut
  `:61-67` then lowest-id tiebreak `:81-84`) and `ResolveIndex::object_by_number` (`src/program/resolve/index.rs:337-364`,
  own-app-shortcut `:345-347` then lowest-id tiebreak `:357-360`). BOTH silently keep the lower `ObjectNodeId`, never
  signal ambiguity. → **Task 1 Step 1 MUST `rg "resolve_object\("` + `rg "object_by_number\("` across `src/` and
  classify EVERY caller** (semantic AL-reference vs indexing/debug) — the fix is incomplete until all semantic callers
  fail closed.
- `resolve_object_ref` (`index.rs:404-458`) → `ObjectRefResolution {Unique|Ambiguous|OutOfClosure|Unresolved}` (`:64-79`)
  reads the RAW indices directly (`objects_by_name`/`objects_by_id`, NOT via `resolve_object`) — so it is already sound
  and is the fail-closed target. Name arm HAS own-app-shadow (`:444-446`); **Id arm does NOT (`:413-431`)** — add it to
  MATCH `object_by_number`'s existing own-app-shortcut (`index.rs:345-347`), i.e. behavior-PRESERVING for the numeric
  path, not a new semantic claim.
- **Caller A lossy-dispatch hazard:** `parsed_type_to_receiver` receives `table_name: String` already lowercased +
  unquoted + temp-stripped by `classify_type_text` (`receiver.rs:286-293`) — so `Record 18` (id) and `Record "18"`
  (name) are INDISTINGUISHABLE at this point. `resolve_table_id`'s existing `table_name.trim().parse::<i64>()` ALREADY
  has this bug; do NOT propagate it — preserve the shape upstream in `ParsedType::Record` (carry a `by_id: Option<i64>`
  / `is_quoted` flag from `classify_type_text`) OR fail-closed on numeric-looking normalized names.

**Bare implicit-`Rec` (Tasks 2–3):**
- `resolve_bare` (`src/program/resolve/resolver.rs:288`): (1) own object (`:296-307`), (2) extension base (`:309-321`),
  **(3) Implicit-Rec — EMPTY TODO `:322-328`**, (4) global builtin (`:329-340`), (5) Unknown.
- REUSE algorithm — `resolve_member`'s `ReceiverType::Record{table}` arm (`resolver.rs:726-790`): `scope = [table_id] +
  index.table_extensions_of(table)` (`index.rs:464-467`), candidate count via `object_has_member_candidate` (`:261-276`),
  **>1 → honest `Unknown`** (`:769-781`). **VISIBILITY CAVEAT (round-1 finding): confirm `table_extensions_of` is
  closure-filtered by the caller's app; if NOT, `resolve_member` today can already resolve a TableExtension method from
  an app the caller doesn't depend on — a latent false-`Source`. The shared helper must take `from_object` and filter.**
- Implicit-Rec table per kind (mirror L3 `call_resolver.rs:757-765`): `Table`→itself; `Page`→`source_table`;
  `TableExtension`→`extends_target`; `PageExtension`→base page `source_table`. Pieces: `ObjectNode.source_table`/`table_no`
  (`node_extract.rs:81-87`), `resolve_source_table_ref` (`receiver.rs:630-642`), `resolve_pageext_base_source_table`
  (`:678-690`).
- Bare-call site: `CalleeShape::Bare { name }` (`extract.rs:28`); `resolve_bare` called WITH the caller's `ObjectNode` at
  `full.rs:300`. **`with`-scope: investigate whether the IR/`RawSiteV2`/routine body tracks enclosing `with` statements
  (`crates/al-syntax` + `extract.rs`); AL binds a bare call inside `with X do` to `X`, NOT `Rec`.**
- L3 semantics (BEHAVIOR reference only): `call_resolver.rs:535-540` + `:746-776` — bare call = implicit `Rec.<proc>()`
  AFTER own-object shadow; Fallback-1.5 (implicit Rec) sits BEFORE Fallback-2 (builtin). **This ordering is L3's choice,
  NOT proven-correct — Task 3 fixtures must compiler-verify it, and fail closed on any table-proc↔builtin/intrinsic collision.**

**Metric surface:** `cdo_full_program_coverage_and_self_reported_metric` (primary `<= 0.030`, measured 2.81%);
`cdo_l3_semantic_audit_no_fresh_wrong` (`genuine_wrong == 0` hard gate, `FRESH_MISSING_CEILING = 110`, measured 102).
`CDO_WS="U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud"`.

## Global Constraints

- Rust edition 2024; toolchain 1.96.0. `rustfmt <file>` per-file — NEVER `cargo fmt`. Stage only named files — NEVER
  `git add -A`. Update `CHANGELOG.md` per task.
- CI gates: `cargo clippy --release --all-features -- -D warnings` (NO `--tests`), `cargo fmt --check`, `cargo test
  --workspace` (NO `CDO_WS`, green). New fixtures under `tests/r0-corpus/`.
- **Soundness is the cardinal rule.** A wrong object/table/proc = false-`Source` = worse than `Unknown`. FAIL CLOSED on
  ANY ambiguity / out-of-closure / unproven-precedence / `with`-scope / builtin-collision. When AL semantics are
  uncertain, decline and defer — never guess.
- **The L3 `genuine_wrong == 0` CDO gate is a REGRESSION BACKSTOP, not a semantic proof** (it can't catch a fresh error
  that L3 shares). Correctness is proven by compiler-backed / hand-verified SYNTHETIC fixtures for each new semantic rule.
- **Measure, don't assume.** Each resolution-changing task RE-MEASURES the CDO `fresh_missing`/rate delta and reports the
  ACTUAL number + a sample of newly-resolved sites ACROSS object kinds (not just the one known example). Resolving FEWER
  than 82 is fine; resolving a WRONG one is fatal.
- Determinism (charter §C8): the shared helper orders any candidate vector by stable `ObjectNodeId`; no hash-iteration
  order in serialized output. No `engine::l3`/`engine::l2` import in `src/program/resolve` except `builtins.rs::global_builtins`.
- **Out of scope (next plans):** Codeunit-`TableNo` `OnRun` implicit-`Rec` (safe to leave `Unknown`; a clean guarded
  follow-up); `with`-scope RESOLUTION (only the fail-closed GUARD is in scope here); the SAME_OBJECT/MIXED `fresh_missing`
  buckets; overload DISPATCH; Report implicit-`Rec`; the span line-offset.

## File / module structure

| File | Responsibility |
|------|----------------|
| `src/program/graph.rs`, `src/program/resolve/index.rs` (modify) | Task 1: make `resolve_object`/`object_by_number` ambiguity-aware (or migrate callers); own-app-shadow in `resolve_object_ref` Id arm. |
| `src/program/resolve/receiver.rs` (modify) | Task 1: route Caller A (shape-preserving) + Caller B onto `resolve_object_ref`; delete `resolve_table_id`. |
| `src/program/resolve/resolver.rs` (modify) | Task 2: visibility-scoped `resolve_in_table_scope` extract. Task 3: `resolve_bare` Step 3 (with/builtin guards). |
| `crates/al-syntax/**` (read; modify only if `with`-tracking must be surfaced) | Task 3: `with`-scope investigation. |
| `tests/r0-corpus/**` + `tests/program_resolve_harness.rs` (create/modify) | Tasks 1/2/3/4: fixtures, characterization tests, CDO gates, ratchets, grep-guard extension. |
| `CHANGELOG.md` + charter memory (modify) | Task 4. |

---

### Task 1: I1 — fix pick-first at the ROOT + own-app-shadow + migrate all semantic callers + grep-guard

**Files:** Modify `src/program/graph.rs`, `src/program/resolve/index.rs`, `src/program/resolve/receiver.rs`; Test
(unit tests for the base functions + `tests/r0-corpus/ws-crossapp-table-collision/` or an equivalent graph/index unit
test — synthetic same-name-across-deps, since same-name tables in ONE closure are AL-illegal to build as a real `.app`).

- [ ] **Step 1: Caller audit (do FIRST).** `rg "resolve_object\(" src tests` + `rg "object_by_number\(" src tests`.
  Classify EVERY caller: semantic AL-object-reference resolution (must fail closed) vs indexing/debug/test. List them in
  the report. The fix is not complete until every semantic caller declines on cross-app ambiguity.
- [ ] **Step 2: Write failing tests** —
  (a) **Base-function ambiguity** unit tests: a synthetic graph where two DEPENDENCY apps declare the same-name Table (and
  the same numeric id) → the ambiguity-aware `resolve_object`/`object_by_number` (or their `resolve_object_ref` replacement)
  DECLINES (`Ambiguous`/`None`), does NOT return the lower id. Own-app declaration present → `Unique(own)` (shadow preserved).
  (b) **Id-arm own-app-shadow** (`resolve_object_ref`): `from`'s own app + a dep both declare Table id N → `Unique(own)`
  (matches `object_by_number`'s existing shortcut); two deps colliding → `Ambiguous`.
  (c) **Caller A shape**: `var R: Record 18` (numeric) → resolves by ID; `var R: Record "18"` (a table literally named
  `18`) → resolves by NAME even if table id 18 exists; if the shape can't be recovered, the numeric-looking name → `None`
  (fail closed), NOT a guessed `Id`.
  (d) **Caller A/B e2e decline**: two deps declare `"Shared Tbl"`; `var R: Record "Shared Tbl"; R.NonBuiltinProc()` →
  DECLINE (`Record{None}` → honest `Unknown`), not pick-first `Source`.
- [ ] **Step 3: Run — fail.**
- [ ] **Step 4: Implement** —
  (i) **Root fix (COMMIT to hardening the base APIs — gpt round-2, no "OR migrate" openness):** change
  `graph::resolve_object` + `index::object_by_number` so they PRESERVE own-app-shadow (a `from`-app declaration wins) but
  return `None` on >1 VISIBLE-in-closure dependency match (never the lowest-id tiebreak). Update every semantic caller (from
  the Step-1 audit) to treat that `None` as honest decline; semantic callers that need the `Ambiguous`/`OutOfClosure`
  distinction use `resolve_object_ref` instead. If a genuinely NON-semantic caller (indexing/diagnostics) needs the old
  pick-first, extract it to a deliberately-named `resolve_object_first_by_stable_id` and grep-guard it OUT of
  `src/program/resolve/**` semantic paths.
  (ii) Add own-app-shadow to `resolve_object_ref`'s `Id` arm (mirror the Name arm `:444-446`).
  (iii) Caller B (TableExtension arm): rewrite on the `resolve_pageext_base_page` template for `ObjectKind::Table`.
  (iv) Caller A (`parsed_type_to_receiver`): thread `from_object`'s `ObjectNodeId` down; carry the object-ref SHAPE from
  `classify_type_text` (extend `ParsedType::Record` to record numeric-id vs quoted-name) → build `ObjectRef::Id`/`Name`
  faithfully → `resolve_object_ref` → `Unique`→`Some`, else `None`. If preserving the shape is disproportionate, fail-closed
  on numeric-looking normalized names (document the small recall loss).
  (v) DELETE `resolve_table_id`.
  (vi) Extend the `resolve_module_has_no_stray_engine_l3_l2_imports` grep-guard family (or add a sibling) to FORBID new
  semantic uses of the pick-first base functions from `src/program/resolve/**` (assert the known-good caller set).
- [ ] **Step 5: Run — pass.** (WITH `CDO_WS`, single tests) `cdo_l3_semantic_audit_no_fresh_wrong` (`genuine_wrong` stays
  0 — I1 is dormant on CDO) + `cdo_full_program_coverage_and_self_reported_metric` (rate unchanged). Confirm no regression.
- [ ] **Step 6: rustfmt + clippy + (no-CDO) test + commit** — `fix(resolve): fail-closed object resolution at the root
  (resolve_object/object_by_number ambiguity-aware) + Id-arm own-app-shadow + shape-preserving declared-var; delete
  pick-first resolve_table_id (I1)`.

---

### Task 2: Visibility-scoped `resolve_in_table_scope` extract (characterization-tested)

**Files:** Modify `src/program/resolve/resolver.rs`, `src/program/resolve/index.rs` (if `table_extensions_of` needs a
closure-filtered variant); Test (characterization + a visibility fixture).

**Interfaces (Produces):** `fn resolve_in_table_scope(from_object: &ObjectNode, table_id: ObjectNodeId, name_lc: &str,
arity: usize, graph, index, body_map) -> Option<(DispatchShape, Vec<Route>)>` — builds `scope = [table_id] + { table
extensions of table that are VISIBLE in from_object's app closure }`, counts candidates via `object_has_member_candidate`,
returns the single `Source` route on exactly 1, honest ambiguous/`Unknown` on >1, `None` on 0. Deterministic ordering
(candidate vectors sorted by stable `ObjectNodeId`).

**Cross-app `Internal`/`Local` visibility (gemini round-2):** when `table_id` (or an extension) is in a DIFFERENT app than
`from_object`, a candidate procedure marked `Access = Internal` (or `Local`) is NOT visible to `from_object` and must be
EXCLUDED from the candidate set — otherwise the helper could mint a false `Source` to a dependency's internal method (which
would not compile in AL) or fabricate a false ambiguity. Task 2 Step 1 determines whether the index tracks procedure access
modifiers: if yes, filter by them; if NO, document the limitation and fail closed (honest `Unknown`) when a cross-app
candidate's visibility is indeterminate rather than assuming it's callable.

- [ ] **Step 1: Investigate + characterize.** Determine whether `index.table_extensions_of` is ALREADY closure-filtered.
  Write characterization tests for the current `resolve_member` Record arm: 0 candidates→fall-through/Unknown, 1 base-table,
  1 visible extension, base+extension collision→Unknown, sibling-extension collision→Unknown, and a NON-visible extension
  (extension in an app NOT in the caller's closure)→must NOT resolve. If the current code resolves the non-visible case,
  that is a pre-existing latent false-`Source` — this task FIXES it (a soundness improvement, measured; not "byte-identical").
- [ ] **Step 2: Run** the characterization tests (some may already pass; the non-visible one likely FAILS today).
- [ ] **Step 3: Extract + visibility-scope.** Lift the scope+cardinality logic from `resolve_member`'s Record arm into
  `resolve_in_table_scope`, taking `from_object` and filtering the extension set by `from_object.id.app`'s closure (add a
  closure-filtered `table_extensions_of` variant if needed). Rewrite the Record arm to call it. Behavior is IDENTICAL
  except the non-visible-extension case now correctly declines (the soundness fix).
- [ ] **Step 4: Run** — (no CDO) `cargo test --workspace` green; (WITH `CDO_WS`) semantic audit — `genuine_wrong` stays 0,
  `fresh_missing`/digest recorded (may change ONLY if CDO had a non-visible-extension resolution, which would be a false-
  `Source` correction — confirm it's a correction, not a regression). If nothing moves, note byte-identical.
- [ ] **Step 5: rustfmt + clippy + commit** — `refactor(resolve): extract visibility-scoped resolve_in_table_scope from
  resolve_member Record arm (closure-filter TableExtensions — fail-closed)`.

---

### Task 3: `resolve_bare` Step 3 — bare implicit-`Rec` (fail-closed: `with`-guard + builtin/intrinsic precedence)

**Files:** Modify `src/program/resolve/resolver.rs` (+ `crates/al-syntax`/`extract.rs` only if `with`-scope must be
surfaced); Test `tests/r0-corpus/ws-bare-implicit-rec/` + no-CDO harness + CDO gate.

**Interfaces (Consumes):** `resolve_in_table_scope` (Task 2); `resolve_source_table_ref`/`resolve_pageext_base_source_table`;
`ObjectNode.source_table`/`extends_target`; the builtin catalog (`is_global_builtin` + the relevant instance/page intrinsics).

- [ ] **Step 1: `with`-scope investigation → a TRI-STATE guard (gpt round-2).** Determine whether the IR / `RawSiteV2` /
  the routine body model tracks enclosing `with X do` statements (grep `crates/al-syntax` + `extract.rs` for `with`). The
  guard Step 3 uses is tri-state, and Step 3 runs ONLY on `NoWithProven`:
  - `NoWithProven` (AST proves the site is NOT inside any `with`, OR the routine provably contains no `with`) → Step 3 may run.
  - `InsideWith` / `HasWith` (AST localizes the site inside a `with`, or the routine is known to contain one) → skip Step 3.
  - `Unknown` (the AST cannot prove absence) → **skip Step 3** (fail closed) — do NOT assume no-`with`.
  If AST-level localization is unavailable, the fallback for `NoWithProven` is a conservative raw-token/text scan of the
  routine for the `with` keyword: a hit → `HasWith` (skip); a clean scan → `NoWithProven`. False positives (over-skipping)
  are fine; a false negative (running Step 3 in an unrepresented `with`) is fatal, so if NEITHER the AST NOR a raw scan can
  prove `NoWith`, decline. (Note: Base App 24 removed `with`, AppSourceCop forbids it → real-world frequency is low, so the
  guard costs little; it exists to stay sound on legacy/ISV inputs.)
- [ ] **Step 2: Write failing + negative + PRECEDENCE fixtures** `tests/r0-corpus/ws-bare-implicit-rec/` (these are the
  correctness proof — compiler-semantics, not L3):
  (a) Table `Customer` `procedure GetName()`; Page `SourceTable = Customer` calls BARE `GetName()` → `Customer.GetName`,
  `Evidence::Source` (today `Unknown`). [MUST NOT be resolvable via Step 1 — the Page has no own `GetName`.]
  (b) **Own-object shadow:** the Page ALSO declares `procedure GetName()` → bare `GetName()` → the PAGE's own (Step 1 wins).
  (c) **Visible TableExtension:** a TableExtension on `Customer` (in a dep the page depends on) `procedure ExtProc()`; bare
  `ExtProc()` in the Page → resolves to the extension's `ExtProc`.
  (d) **NEGATIVE — sibling-extension ambiguity:** two visible TableExtensions each `procedure Dup()` (same arity) → bare
  `Dup()` → honest `Unknown`.
  (e) **NEGATIVE — builtin collision:** a Table proc named `StrLen(Text)`; a Page on it calls bare `StrLen(x)` → **honest
  `Unknown`** (fail-closed on the table-proc↔builtin collision — do NOT assume table wins; a comment cites this as the
  conservative default pending a compiler-verified precedence).
  (f) **NEGATIVE — page-intrinsic collision:** a Table proc named `Update()`; a Page on it calls bare `Update()` → honest
  `Unknown` (bare `Update` could be the page intrinsic; fail closed).
  (g) **NEGATIVE — `with`-block:** a bare `GetName()` inside `with OtherRec do begin … end` (OtherRec a different record
  var) → honest `Unknown` (NOT `Customer.GetName`). [Only assert this if Step-1 found `with` is representable; else assert
  the coarse routine-level guard.]
  (h) **NEGATIVE — no implicit table:** a plain Codeunit (no `TableNo`) bare `Foo()` (not own, not builtin) → `Unknown`.
  (i) **Shadow-guard (NOT a Step-3 proof):** Table `Customer` bare `Recalc()` inside Customer's OWN trigger → resolves via
  Step 1 (own object) — assert it resolves to `Customer.Recalc` but document it does NOT exercise Step 3.
  (j) **PageExtension base-vs-SourceTable precedence (gpt round-2):** a PageExtension whose base Page declares `procedure
  Foo()` AND whose SourceTable declares `procedure Foo()`; a bare `Foo()` in the PageExtension → resolves to the BASE PAGE's
  `Foo` via Step 2 (extension base runs BEFORE Step-3 implicit-Rec). Pin this ordering IF it is intended/compiler-consistent;
  otherwise assert current behavior + document it as pre-existing (Task 3 does NOT claim to prove this precedence).
  (k) **Strict-kind negative:** a Report/XmlPort with a bare call whose name matches a proc on some table → stays honest
  `Unknown` (Step 3 structurally excludes these kinds; Report implicit-`Rec` is dataitem-scoped, a future task).
- [ ] **Step 3: Run — fail** ((a)/(c) `Unknown` today; the negatives must already hold or reveal a gap).
- [ ] **Step 4: Implement Step 3** in `resolve_bare` (between `:321` and `:329`):
  (0) **strict ObjectKind (gemini round-2):** structurally `match from_object.kind` on ONLY `{Table, Page, TableExtension,
  PageExtension}`; every other kind (Codeunit, Report, XmlPort, Query, …) → skip Step 3 (fall through) — no accidental leakage.
  (1) **`with`-guard** — run Step 3 ONLY when Step-1's tri-state is `NoWithProven`; `HasWith`/`Unknown` → skip.
  (2) compute the implicit-Rec table id by kind (`Table`→self; `Page`→`resolve_source_table_ref`;
  `TableExtension`→`resolve_object_ref(Table, extends_target)`; `PageExtension`→`resolve_pageext_base_source_table`). If no
  unique table id → fall through.
  (3) call `resolve_in_table_scope(from_object, table_id, name_lc, arity, …)` (visibility-scoped, Task 2).
  (4) **builtin/intrinsic PROBE-THEN-DECIDE (gpt round-2 — NOT skip-before-probe):** if `name_lc` is a global builtin OR a
  bare-callable page/instance intrinsic AND step (3) found ANY same-name+arity table candidate → return honest `Unknown`
  (the collision is unproven precedence — fail closed, do NOT emit `Catalog`). If it's a builtin/intrinsic with NO table
  candidate → fall through to Step 4 (`Catalog`). If it's NOT a builtin/intrinsic → return step (3)'s result (`Some(route)`
  = Source or honest-ambiguous `Unknown`; `None` → fall through). Own-object (Steps 1–2) already shadowed. Everything
  fail-closed.
- [ ] **Step 5: Run — pass** (ALL incl. negatives). CDO gate (WITH `CDO_WS`, single tests): `cdo_full_program_coverage_and_self_reported_metric`
  (record the ACTUAL rate drop from 2.81%) + `cdo_l3_semantic_audit_no_fresh_wrong` (`fresh_missing` — record the ACTUAL
  drop from 102; `genuine_wrong` MUST stay 0). Hand-adjudicate a SAMPLE across object kinds (incl. the report's `Page
  6175272` bare `GetReportSelection()`→table 6175283). Report the measured delta — do NOT expect 82. Deterministic.
- [ ] **Step 6: rustfmt + clippy + (no-CDO) test + commit** — `feat(resolve): resolve bare implicit-Rec calls
  (resolve_bare Step 3, with-guarded + builtin-collision-fail-closed, visibility-scoped) (bare-implicit-rec)`.

---

### Task 4: Re-measure, tighten ratchets, CHANGELOG + charter memory

**Files:** Modify `tests/program_resolve_harness.rs` (ratchets), `CHANGELOG.md`, charter memory; Test.

- [ ] **Step 1: Full re-measure** (WITH `CDO_WS` + `ENFORCE_CDO_WS=1`, single tests): record FINAL primary/whole-program
  rate + `unknown` count + `fresh_missing` + per-bucket residual + `genuine_wrong`. Capture `aldump --program-call-graph-stats`.
- [ ] **Step 2: Tighten ratchets to the measured floor** (never loosen): lower `primary_rate <=` (0.030) if dropped; lower
  the `unknown` COUNT ceilings (520); lower `FRESH_MISSING_CEILING` (110) to the new residual + margin; keep `genuine_wrong
  == 0` + the `fresh_wrong` ceiling. Dated comments.
- [ ] **Step 3: Run** — (no CDO) green incl. grep-guards; (WITH `CDO_WS`) all gates green under tightened ratchets,
  deterministic, `checked_sites > 0`.
- [ ] **Step 4: CHANGELOG (honest)** — I1 root-cause fix (ambiguity-aware base functions + own-app-shadow + shape-preserving
  declared-var; deleted `resolve_table_id`); Task-2 visibility-scoped extract; `resolve_bare` Step 3 (bare implicit-`Rec`,
  `with`-guarded, builtin-collision-fail-closed) with MEASURED before/after `fresh_missing` + rate. State what stays honest
  (with-scope resolution, Codeunit-`TableNo`, table↔builtin precedence pending compiler proof, SAME_OBJECT/MIXED buckets).
  No `engine::l3` import (grep-guarded).
- [ ] **Step 5: Charter memory** — append a concise entry to `C:\Users\SShadowS\.claude\projects\U--Git-al-call-hierarchy\memory\semantic-intelligence-charter.md`
  (I1 closed at root; bare implicit-`Rec` resolved fail-closed; real-`unknown` 2.81%→[measured]%; next: with-scope,
  Codeunit-`TableNo`, table/builtin precedence proof, SAME_OBJECT/MIXED). Update `MEMORY.md` pointer if warranted.
- [ ] **Step 6: Commit** — `docs(resolve): follow-up complete — I1 root fail-closed + bare implicit-Rec; real-unknown
  2.81%→[measured]% (follow-up Task 4)`.

---

## Roadmap — beyond this plan

Codeunit-`TableNo` `OnRun` implicit-`Rec` (guarded to the OnRun trigger); `with`-scope RESOLUTION (bind bare calls to the
`with` record var, not just guard); a compiler-verified table-proc↔builtin PRECEDENCE (relax the fail-closed collision
guard once proven); the SAME_OBJECT (12) + MIXED (8) `fresh_missing` buckets; same-arity-type overload DISPATCH; Report
implicit-`Rec` (dataitem block-scope); the span line-offset. North-star (charter §8): drive workspace-originated
real-`unknown` to its provably-dynamic residual.
