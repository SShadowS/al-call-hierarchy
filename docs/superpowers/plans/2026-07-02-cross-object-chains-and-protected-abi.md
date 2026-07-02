# Cross-object call-result chains + protected-ABI soundness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

> Status: **v2.1** (round 2: gemini GO, gpt GO-WITH-CHANGES — folded: the ABI arity-PROOF (tri-state arity, missing
> `Parameters` = unknown never zero, unknown-arity never emits, wrong-arity single-candidate negatives); no name-only
> catalog-emission path for new entries; Key-facts bullets synced to the corrected task text; Name+Id validation covers
> ABI-tier `Routine(rid)` routes too; quote-bearing Subtype names strictly DECLINE; single-impl interface prefers the
> interface's own signature. Round 1 = both GO-WITH-CHANGES; criticals folded: SymbolOnly selection gets FULL source-tier
> discipline — never first-visible/order-dependent; the T3 ABI-prefix uniqueness guard — same-name/same-arity ABI overloads
> decline, closing both the AbiSymbol re-derivation divergence AND the degraded-param-fingerprint mis-pick without fixing
> param subtypes now; Name+Id cross-validation with Id-only→decline; FieldRef.Value non-chainable; name-only scan `.any()`
> + never-emits audit; changed-target adjudication; ABI-Variables[]-unmodeled proof). Fifth resolution arc (master `8a484d4`, CDO real-`unknown` primary 1.82% /
> `unknown=329`, whole-program 0.76%, `genuine_wrong=0`). The two opus-ranked roadmap items, both grounded this session:
> 1. **Soundness (the flagged latent hole):** the ABI/SymbolOnly tier mislabels dependency `protected` members as `Public`
>    (the schema field exists but is unparsed) AND `resolve_in_object`'s SymbolOnly branch takes `candidates.first()` with NO
>    visibility check — a false-`Source`/`Catalog` vector for any ABI-only dep (measured DORMANT on CDO: its one true
>    SymbolOnly unit is an empty shell; every real dep is EmbeddedSource — fixture-proven like the dormant Task-3 resolver).
> 2. **Burndown (the bulk of the residual):** `compoundReceiver=156` bucketed by real CDO shape: (a) `Var.Method().X()`
>    cross-object call-result chains (source + ABI prefix) — the missing Phase-A arm; (b) Xml framework chains — pure
>    `framework_returns.rs` table extension; (d) `RecordRef`/`FieldRef`/`KeyRef` chains — a new typed-return family; plus a
>    stale `HTTPCONTENT` member catalog. (c) `Rec."Field".X()` record-field chains stay DEFERRED (no table-field index —
>    its own future modelling task).

**Goal:** Close the protected-ABI false-`Source` vector (fail-closed, fixture-proven) AND resolve the cross-object /
framework-family chain shapes, driving CDO real-`unknown` DOWN — **adding zero false-`Source`/`Catalog` claims**
(`genuine_wrong` stays 0), every new resolution proven by compiler-semantics fixtures + EXHAUSTIVE hand-adjudication of the
CDO delta.

**Architecture:** Task 1 (protected-ABI soundness) FIRST — the chain arm dispatches type-queries through `resolve_in_object`,
which must be visibility-correct for SymbolOnly before chains start resolving via ABI paths. Task 2 adds the structured ABI
return-type carrying (lossless `Subtype`; resolution-neutral until consumed). Task 3 adds the cross-object chain arm
(prefix typed via a pure `resolve_member` type-query — mirrors the landed `resolve_bare` pattern). Task 4 extends the leaf
tables (Xml, RecordRef-family, HTTPCONTENT) — pure validated data + one small arm. Task 5 measures + adjudicates + closes.
All resolvers fail closed; reuse the existing fail-closed machinery, never hand-roll.

**Tech Stack:** Rust (edition 2024, toolchain 1.96.0). No new dependency. No `engine::l3`/`engine::l2` import in
`src/program/resolve` except `builtins.rs::global_builtins` (grep-guarded).

**Source of truth:** the two grounding reports (this session, file:line + real-`.app`-verified) + master `8a484d4` + the
charter (§5 taxonomy, §6 no-false-certainty, §8 metric).

## Key facts grounding this plan (verified on `8a484d4`)

**Protected-ABI (Task 1):**
- `SymbolReference.json` DOES carry the marker: `"IsProtected":true` on `Methods[]` (System App: 10 occurrences, exactly
  matching its embedded source's 10 `protected procedure`s — 1:1 verified) and `"Protected":true` on `Variables[]`. The fix
  is pure parsing.
- Touch points: `symbol_reference.rs` — `AbiRoutine` (:53-68) + `RawMethod` (:170-184, add `#[serde(rename="IsProtected")]`)
  + `parse_method` (:557-589, beside the existing `is_local`/`is_internal` at :585-586). `abi_ingest.rs` — KEEP the
  `is_local||is_internal` skip (:263-265); change `access: Access::Public` (:283) → `if routine.is_protected
  { Access::Protected } else { Access::Public }`. **Carry-Protected, NOT drop**: AL lets an extension of the dep object call
  its `protected` members; the per-candidate rule (`Protected → self OR index.object_extends`) already exists and
  `object_extends` is tier-agnostic (resolves a workspace extension of a SymbolOnly base correctly).
- **THE fix site — three SymbolOnly short-circuits** (all in `resolver.rs`):
  1. `resolve_in_object`'s SymbolOnly branch (`:257-265`) takes `candidates.first()` → `make_routine_route` with NO
     visibility check (reached from the Object/SelfObject arms AND the interface-implementer fan-out `:1550-1566`). Must
     consult per-candidate visibility before emitting.
  2. `object_has_visible_member_candidate` (`:489-491`): `if obj_tier == SymbolOnly { return true; }` — must fall through
     to the per-candidate match. NUANCE: SymbolOnly existence checks are ARITY-DEFERRED (any name match, `:407-409`), so the
     visibility fallthrough must scan candidates by NAME ONLY (not the arity-filtered list) or a real ABI arity mismatch
     would wrongly suppress a legitimately-visible candidate.
  3. `access_exclusion_reason` (`:525-527`): SymbolOnly early-return `None` — compute the real `ProtectedNotVisible` reason.
  Plus stale doc comments (`:233-235`, `:434-443`, `:524-525`) asserting "SymbolOnly is always Public".
- CDO realized exposure measured **0** (the only true SymbolOnly unit, `Microsoft_Application_28.*.app`, is a 376-byte empty
  shell; all real deps are EmbeddedSource) → this task is CDO-neutral (1.82%/329 byte-identical); proof = fixtures.

**ABI return-type (Task 2):**
- `abi_ingest.rs:294-298` hard-sets ABI `RoutineNode.return_type = None` (test `abi_routine_return_type_is_discarded` :661 —
  the prior plan's explicit deferral; this task REVERSES it, flip the test).
- The JSON field is NESTED: `"ReturnTypeDefinition":{"Name":"Codeunit","Subtype":{"Name":"Http Content","Id":2354}}`
  (real System App: `HttpResponseMessage.GetContent(): Codeunit "Http Content"`). `RawTypeDef` (`symbol_reference.rs:149-158`)
  deserializes only `Name`/`Temporary` — **`Subtype` is silently dropped by serde**, so naive wiring would yield the bare
  keyword `"Codeunit"` (an empty-name object ref — fails closed but silently). Fix: add
  `subtype: Option<RawSubtype{name, id}>` to `RawTypeDef`; reconstruct source-shaped text — `Codeunit "Http Content"`
  (NAME-preferred quoted; **Id-only or quote-bearing Name → DECLINE, `None`** — never a numeric fallback, never escaping) —
  so `classify_type_text`/`parsed_type_to_receiver` parse it identically to source; carry the structured `(name, id)` pair
  for Task-3 cross-validation. Bare framework/value returns (`{"Name":"HttpHeaders"}`, no Subtype) already complete — pass
  through unchanged.
- (Pre-existing, OUT OF SCOPE, note in CHANGELOG: `AbiParameter.type_text` shares the same `Subtype` drop → ABI
  `param_sig_key`/`sig_fp` fingerprints are computed over degraded text — a separate follow-up.)

**Cross-object chain arm (Task 3):**
- `resolve_member(receiver, method_lc, arity, from_object, graph, index, body_map) -> (DispatchShape, Vec<Route>)`
  (`resolver.rs:1308`) is PURE (no emission, read-only params; same purity contract as the already-reused `resolve_bare`)
  and a strict downstream LEAF (grep-verified: it never calls back into receiver inference → recursion depth is bounded by
  the finite expression tree; no cycle risk).
- Type-query guard (mirror the landed Step-5 guard `receiver.rs:1024-1029`): require EXACTLY ONE route; target
  `RouteTarget::Routine(rid)` → read `graph.routines[rid].return_type` (Name+Id cross-validation applies when the node is
  ABI-tier); target `RouteTarget::AbiSymbol{key}` (a true SymbolOnly body-map miss — the Route does NOT carry a
  `RoutineNodeId`) → the ABI-PREFIX UNIQUENESS GUARD (see Task 3 Step 3 — arity-PROVEN, exactly-one-before-param-narrowing,
  never a trusting `(object,name,arity)` re-lookup under overloads). `Builtin`/`Unresolved`/≠1 routes → decline.
  Interface-typed prefix with >1 implementer → the polymorphic fan-out yields >1 route → the guard declines (conservative,
  never pick one return type).
- Non-scalar / `Some` return → `classify_type_text` → `parsed_type_to_receiver` (from_object-scoped, cross-app fail-closed,
  interface-safe — all inherited). Evidence taxonomy: NO new kind needed (Phase A receiver-typing is invisible to Phase B
  dispatch — verified).

**Leaf tables (Task 4) — the other real CDO buckets:**
- Bucket (b) Xml chains (≥10 real CDO sites: `XmlElement.Create(...).AsXmlNode()`, `Node.AsXmlElement().GetChildNodes()`,
  `ChildNode.AsXmlText().Value`, …): base already types `Framework(Xml)`; `framework_returns.rs` simply lacks Xml entries
  (deliberate Task-4 scope cut). Pure table extension, MS-Learn provenance per the file's own doctrine.
- Bucket (d) RecordRef-family chains (3+ real sites: `SourceRecRef.KeyIndex(1).FieldCount`, `KeyRef.FieldIndex(1).Value`):
  `infer_compound_member_receiver` (`receiver.rs:870-872`) hard-matches `ReceiverType::Framework` — `RecordRef`/`FieldRef`/
  `KeyRef` are SIBLING ReceiverType variants that decline before any table. `member_catalog.rs`'s RECORDREF/KEYREF sets are
  membership-only (no return types). Needs a small new typed-return table for `(RecordRef|FieldRef|KeyRef, member_lc,
  is_method, arity) → ReceiverType` + an arm for those variants — same mechanism, distinct family.
- The `HTTPCONTENT` phf set (`member_catalog.rs:141-143`) is STALE: only `clear/getheaders/issecretcontent/readas/writefrom`
  — the real ABI methods `AsText`/`AsBlob`/`AsInStream`/`AsJson*` (SymbolReference-verified) are missing, so even a typed
  `HttpContent` receiver can't resolve them. Extend with MS-Learn/SymbolReference-validated entries.

**Deferred (next plan):** bucket (c) `Rec."Field".X()` record-field chains — `ObjectNode` has NO table-field index
(`FieldDecl` parsed, zero consumers under `src/` — re-verified); needs its own field-modelling task. `untrackedReceiver=91`;
the honest-taxonomy reclassification (OverloadAmbiguous 56 / MemberNotFound 25); ABI param-fingerprint degradation.

**Metric gates:** `cdo_full_program_coverage_and_self_reported_metric` (primary ceiling `0.019`, measured 1.82% /
`unknown=329`, count ceiling 337); `cdo_l3_semantic_audit_no_fresh_wrong` (`genuine_wrong == 0`, `FRESH_MISSING_CEILING=10`,
`FRESH_WRONG_CEILING=149`); `sum(unknownByReason) == unknown`.
`CDO_WS="U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud"`. Run CDO tests SINGLY (parallel L3 assemblies crash).

## Global Constraints

- Rust edition 2024; toolchain 1.96.0. `rustfmt <file>` per-file — NEVER `cargo fmt`. Stage only named files — NEVER
  `git add -A`. Update `CHANGELOG.md` per task.
- CI gates: `cargo clippy --release --all-features -- -D warnings` (NO `--tests`), `cargo fmt --check`, `cargo test
  --workspace` (NO `CDO_WS`, green). New fixtures under `tests/r0-corpus/`.
- **Soundness is the cardinal rule.** A false `Source`/`Catalog` (wrong target claimed) is worse than an honest `Unknown`.
  Task 1 makes ABI resolution MORE fail-closed (CDO-neutral here; fixtures prove it). Tasks 3–4 type a receiver ONLY on a
  UNIQUE, statically-proven type; decline on ANY ambiguity, ≠1 route, polymorphic fan-out, scalar/None return, cross-app
  duplicate, table-miss, unvalidated entry. When in doubt, `Unknown`.
- **Reuse the fail-closed machinery; do NOT hand-roll.** Prefix member-calls type via the PURE `resolve_member` (inherits
  per-candidate access + friend model + overload declines + polymorphic fan-out). Return-type→receiver via
  `classify_type_text` → `parsed_type_to_receiver`. Object identity via `ObjectRef` (quoted-Name vs numeric-Id shape
  preserved — reconstruct ABI type text in source shape).
- **Operate on the parsed AST** (`ExprKind::Call{function: Member{...}}`) — never string-split `receiver_text`.
- **`genuine_wrong == 0` is a REGRESSION backstop, NOT a semantic proof.** Each new resolution class is proven by
  compiler-semantics fixtures (positives + fail-closed negatives, PROOF.md spec-stated if no compiler) AND EXHAUSTIVE
  hand-adjudication of the CDO delta (every newly-`Resolved`/`Catalog` chain edge + every new table entry hit — NOT a
  sample).
- Determinism (charter §C8). Per-entry provenance on every new table entry (validated BC runtime version; uncertain →
  OMITTED). Tables keyed by `(family/kind, member_lc, is_method, arity)`.
- Ratchets move DOWN with measured wins; a justified soundness RISE needs a dated note; never loosen to hide a regression.

## File / module structure

| File | Responsibility |
|------|----------------|
| `src/engine/deps/symbol_reference.rs` (modify) | T1: `IsProtected` parsing. T2: `RawTypeDef.subtype` + structured return carrying. |
| `src/program/abi_ingest.rs` (modify) | T1: `Access::Protected` on ABI nodes. T2: populate `RoutineNode.return_type` (source-shaped text). |
| `src/program/resolve/resolver.rs` (modify) | T1: the three SymbolOnly visibility fixes + doc updates. |
| `src/program/resolve/receiver.rs` (modify) | T3: the cross-object chain arm (resolve_member type-query). T4: the RecordRef-family arm. |
| `src/program/resolve/framework_returns.rs` (modify) | T4: Xml entries (provenanced). |
| `src/program/resolve/member_catalog.rs` or new module (modify) | T4: the RecordRef-family typed-return table + HTTPCONTENT extension. |
| `tests/r0-corpus/**` + `tests/program_resolve_harness.rs` (create/modify) | T1–T5 fixtures (incl. probe `.app`s with `IsProtected` + nested `Subtype`). |
| `CHANGELOG.md` + charter memory (modify) | T5. |

---

### Task 1: Protected-ABI soundness — parse `IsProtected`, carry `Access::Protected`, close the SymbolOnly visibility short-circuits

**Files:** Modify `src/engine/deps/symbol_reference.rs`, `src/program/abi_ingest.rs`, `src/program/resolve/resolver.rs`;
Test `tests/r0-corpus/ws-protected-abi/` (a probe `.app` whose SymbolReference.json has `"IsProtected":true` members) +
no-CDO harness + CDO gate.

- [ ] **Step 1: Write failing + control fixtures.** Probe `.app` (SymbolOnly: NO embedded source) exposing an object with
  `protected procedure P()`, `procedure Pub()`, `internal procedure I()`, `local procedure L()` in its SymbolReference.json.
  (a) workspace object calls dep `P()` via Object receiver → honest `Unknown(ProtectedNotVisible)` (today: false route via
  the SymbolOnly `candidates.first()`); (b) CONTROL: `Pub()` → resolves (Abi/Opaque as today); (c) a workspace
  `TableExtension`/`PageExtension` extending the dep object calls `P()` → RESOLVES (extension sees base protected —
  carry-Protected not drop); (d) CONTROL: `I()`/`L()` still absent (ingestion drop unchanged); (e) interface-impl fan-out to
  a SymbolOnly implementer respects the same visibility; (f) **mixed-arity mixed-access overloads** — dep declares
  `protected procedure GetWorker()` (arity 0) AND `procedure GetWorker(ID: Integer)` (public, arity 1): an external arity-0
  call must go honest `Unknown` (NOT silently select the visible 1-arity sibling — order/visibility-dependent selection is a
  false-`Source` vector); an external arity-1 call resolves to the public overload; (g) **name-only-scan wrong-arity
  control** — a visible public candidate with the right name but wrong arity: the existence boolean may be true, but NO
  `Catalog`/resolved edge is emitted for the unmatched-arity call (the boolean is existence/diagnostics only, never edge
  evidence); (h) **stranger-extension identity negative** — a workspace object with the SAME id/name but a different
  kind/app that does NOT genuinely extend the ABI base must NOT see `P()` (`object_extends` is identity+kind-exact — the
  landed direct+kind-compatible predicate; pin it against a SymbolOnly base). Assert exact pre-fix wrong route for (a)/(f).
- [ ] **Step 2: Run — fail** ((a) resolves today).
- [ ] **Step 3: Implement.** (i) `symbol_reference.rs`: `RawMethod` + `AbiRoutine` gain `is_protected`
  (`#[serde(rename="IsProtected")]`, default false); `parse_method` maps it beside `is_local`/`is_internal`. (ii)
  `abi_ingest.rs`: keep the local/internal skip; survivors get `access: if is_protected { Access::Protected } else
  { Access::Public }`; extend the `local_and_internal_routines_skipped`-family test with the protected case (kept, as
  Protected, not dropped/Public). (iii) `resolver.rs` — fix the three short-circuits with FULL SOURCE-TIER SELECTION
  DISCIPLINE (round-1 critical: "first visible" is order-dependent and unsound):
  - `resolve_in_object`'s SymbolOnly branch: filter candidates by visibility, apply the SAME arity/overload rules as the
    source tier (incl. the landed overload-narrowing guard: if access-filtering removed a same-name sibling and >1 existed
    pre-filter with unproven args → `Unknown(OverloadAmbiguous)`); EMIT ONLY when the final candidate set is EXACTLY ONE —
    never `candidates.first()` on a multi-candidate set, never an order-dependent pick. 0 visible → `Unknown` with the
    access reason. ARITY IS TRI-STATE (round-2): known `n` (from `Parameters[].len()`, count unaffected by the param-subtype
    degradation) participates in exact matching; a missing/unparsed `Parameters` field is UNKNOWN arity — never zero — and
    an unknown-arity candidate NEVER emits an edge (diagnostics only). If the branch cannot enforce arity at its call site
    and >1 visible same-name candidates exist, it MUST fail closed, not pick. Fixture: a SINGLE visible public `Get(ID)`
    with an external `Get()` (arity-0) call → NO emit (exactly-one-same-name is insufficient at the wrong arity).
  - `object_has_visible_member_candidate`'s SymbolOnly short-circuit → a NAME-ONLY per-candidate visibility scan that is
    `.any()` over ALL same-name candidates (never first-match — a protected first sibling must not hide a public one), on
    the UN-arity-filtered list. AUDIT every caller of this boolean: it is existence/diagnostics input only and must never
    directly justify an emitted edge (fixture (g) pins this).
  - `access_exclusion_reason` computes the real reason for SymbolOnly. Update the stale "always Public" doc comments.
  (iv) One-line verification (round-1 I8): grep/prove ABI `Variables[]` are NOT ingested into any route/receiver path
  (`abi_ingest` ingests Methods only) — record it in the report; if they ARE modeled anywhere, escalate (the protected-var
  deferral would be unsound).
- [ ] **Step 4: Run — pass** (all incl. controls). CDO (WITH `CDO_WS`, SINGLE tests): `cdo_l3_semantic_audit_no_fresh_wrong`
  (`genuine_wrong` stays 0) + `cdo_full_program_coverage_and_self_reported_metric` — expected BYTE-IDENTICAL 1.82%/329.
  NOTE (round-1 I3): byte-identical is EMPIRICAL (CDO's true-SymbolOnly surface is empty), not logically guaranteed by the
  selection-logic change — on any workspace with real SymbolOnly candidates this task legitimately changes output. Emit a
  quick diagnostic enumerating non-empty SymbolOnly objects on CDO (expected: none) so any metric movement is immediately
  attributable. If anything moves, STOP and adjudicate.
- [ ] **Step 5: rustfmt + clippy + (no-CDO) test + commit** — `fix(resolve): protected-ABI soundness — parse IsProtected,
  carry Access::Protected, per-candidate visibility on the SymbolOnly path (Task 1)`.

---

### Task 2: Structured ABI return types — carry `Subtype`, populate `RoutineNode.return_type` (source-shaped, resolution-neutral)

**Files:** Modify `src/engine/deps/symbol_reference.rs`, `src/program/abi_ingest.rs`; Test unit + probe `.app` + CDO gate.

- [ ] **Step 1: Write failing tests.** (a) parsing a SymbolReference.json method with
  `"ReturnTypeDefinition":{"Name":"Codeunit","Subtype":{"Name":"Http Content","Id":2354}}` yields an ABI routine whose
  reconstructed return text is `Codeunit "Http Content"` (quoted name — source-shaped) AND retains the structured
  `(name, id)` pair for downstream cross-validation; (b) `{"Name":"HttpHeaders"}` (no Subtype) → `HttpHeaders` unchanged;
  (c) **Subtype with Id but NO Name → `None`** (DECLINE — round-1 critical: AL object ids are NOT cross-app unique; a bare
  numeric reconstruction could resolve to the WRONG app's object; fail closed rather than synthesize); (d) the ingested
  `RoutineNode.return_type == Some(...)` for ABI routines with a usable Name (FLIP the `abi_routine_return_type_is_discarded`
  test to its positive); (e) NEUTRALITY: CDO byte-identical (nothing consumes ABI return_type yet — Task 3 does);
  (f) FORMAT LANDMINES (all fail-closed, round-1 M2 + round-2 gemini): a Subtype Name containing a quote character →
  **strictly DECLINE to `None`** (never escape — downstream text classification must never see synthesized escaping); a
  namespace-qualified Name (if present in the schema — carry or decline, never truncate); a generic/container return
  (`List of [X]`, `Dictionary of [...]`, arrays) → pass through as-is (scalar-declined later) or `None` — never approximate.
- [ ] **Step 2: Run — fail.**
- [ ] **Step 3: Implement.** `RawTypeDef` gains `subtype: Option<RawSubtype{name: Option<String>, id: Option<i64>}>`
  (serde-renamed to the JSON keys); reconstruct source-shaped type text (Name-preferred quoted; **Id-only → None/decline**;
  bare pass-through) AND carry the structured `(name, id)` pair on the `AbiRoutine` so Task 3 can CROSS-VALIDATE: when both
  Name and Id are present, the object the name resolves to must ALSO carry that declared id — a mismatch → decline (round-1
  C4: name-or-id alone is not proof; validate all available identity). `abi_ingest.rs` populates `RoutineNode.return_type`
  from the reconstructed text (replacing the `None` hard-set) and keeps the structured pair available (a parallel field or
  an ABI-side lookup — implementer's choice, but the cross-validation data must reach Task 3). Note the param-fingerprint
  sibling gap in CHANGELOG as out-of-scope follow-up. `RoutineNode.return_type` remains non-serialized (prior-plan
  invariant) — confirm no golden path reads it.
- [ ] **Step 4: Run — pass** + full workspace green + CDO byte-identical (1.82%/329; this task is resolution-neutral — if a
  golden or count moves, STOP).
- [ ] **Step 5: rustfmt + clippy + commit** — `feat(resolve): carry structured ABI return types (Subtype) into
  RoutineNode.return_type — source-shaped, resolution-neutral (Task 2)`.

---

### Task 3: Cross-object call-result chains — `Var.Method().X()` via a pure `resolve_member` type-query, fail-closed

**Files:** Modify `src/program/resolve/receiver.rs` (the new Phase-A arm); Test `tests/r0-corpus/ws-cross-object-chain/`
(incl. an ABI-prefix probe `.app` with nested Subtype) + CDO gate.

- [ ] **Step 1: Write failing + fail-closed-negative fixtures.** POSITIVES: (a) source prefix —
  `Retriever.FindProfile(X).Code()`-style… use a method-call leaf: `Helper.GetCustomer(No).Name()` where `Helper: Codeunit
  CRHelper` declares `procedure GetCustomer(No: Code[20]): Record Customer` and `Name` is a Customer proc → final edge
  `Source` to `Customer.Name`, exact id; (b) ABI prefix — `Response.GetContent().ReadAs(...)` where `Response: Codeunit
  "Http Response Message"` is a SymbolOnly probe dep whose `GetContent` carries the nested Subtype → the chain types
  `Codeunit "Http Content"` and the leaf resolves (Abi/Catalog as appropriate); (c) chain into the friend/access rules —
  the leaf call still respects per-candidate visibility (an `internal` leaf cross-app non-friend → `Unknown`); (d)
  **single-implementer interface prefix SUCCESS control** — `IVar.Method().X()` where the interface has EXACTLY ONE
  implementer in the closure: 1 route → the guard accepts → the chain types by that implementer's return type (which AL
  guarantees matches the interface signature) and the leaf resolves. NEGATIVES (→ `Unknown`): prefix resolves
  polymorphically (interface receiver, >1 impl — conservative decline); prefix ≠1 route / builtin-only / `Unresolved`;
  **ABI same-name overloads with different returns** (the probe dep declares `Get(X: Codeunit A): Codeunit RA` and
  `Get(X: Codeunit B): Codeunit RB` — same name; the ABI-prefix uniqueness guard must decline rather than pick, because ABI
  param types are degraded and cannot disambiguate); scalar or `None` return; `parsed_type_to_receiver` ambiguous (two deps
  define the return-type name); **Name+Id cross-validation mismatch** (the Subtype's id disagrees with the object the name
  resolves to → decline); DEFERRED `Rec."Field".X()` record-field chain stays `Unknown`; a 3-level chain whose middle hop
  fails types → `Unknown` (no partial guessing); a same-named FIELD/property on the base (non-procedure member — the arm is
  procedure-call-form only) → `Unknown`.
- [ ] **Step 2: Run — fail** ((a)/(b) are `Unknown`/`compoundReceiver` today).
- [ ] **Step 3: Implement (AST-based).** In the compound Phase-A step: when the receiver `Expr` is `ExprKind::Call{function,
  args}` and `function` is `ExprKind::Member{base, member}` — STRICTLY the procedure-CALL form (a bare `Member` without the
  Call wrapper is a field/property access, NOT this arm — decline; round-1 I7): (1) type the BASE via the existing
  AST-native `infer_receiver_type_for_expr` (vars/params/globals/Rec/framework — recursion bounded by the expr tree);
  (2) if base types to `Object`/`Record`/`SelfObject`/`Interface` (Framework → Step 6's table; untyped/Unknown → decline):
  call `resolve_member(base_ty, member_lc, <arity via the SAME arity matcher dispatch uses — not a second args.len() model;
  round-1 M1>, from_object, …)` as a TYPE-QUERY; (3) guard: EXACTLY ONE route; `RouteTarget::Routine(rid)` →
  `graph.routines[rid].return_type`; `RouteTarget::AbiSymbol{…}` → **the ABI-PREFIX UNIQUENESS GUARD (round-1 C1+C2,
  round-2 arity-PROOF)**: the Route carries no routine id and ABI param types are degraded (the Subtype drop on
  parameters), so re-derivation may NOT trust a `(object, name, arity)` re-lookup under overloads. Prefer carrying the
  selected routine identity on the route / a shared selection helper; failing that, ALL of: the same arity matcher as
  dispatch; candidate arity KNOWN (tri-state — a missing/unparsed `Parameters` field is UNKNOWN arity, never zero; unknown
  arity NEVER emits); exactly ONE visible same-name candidate at the dispatch arity BEFORE any param narrowing (same-name/
  same-arity siblings → decline; a unique known-arity candidate is by construction the one `resolve_member` selected);
  read ITS `return_type`; anything else → decline; (4) `Some(non-scalar)` → `classify_type_text` → `parsed_type_to_receiver`
  → the receiver type, WITH the Task-2 Name+Id cross-validation applied to EVERY ABI-sourced return type — whether the
  route target was `AbiSymbol` OR an ABI-tier `Routine(rid)` (keep the structured pair reachable by `RoutineNodeId`/side
  map; metadata expected-but-missing for an ABI object return → decline); the leaf member resolution proceeds normally
  (Phase B unchanged — no new evidence kind). Every decline → `Unknown` (reason stays `CompoundReceiver`). ADD the
  wrong-arity ABI negative: `Obj.Get().X()` where SymbolReference has only `Get(ID: Integer): Codeunit Ret` → `Unknown`
  (a single visible same-name candidate at the WRONG arity must not emit). Single-implementer interface prefix: PREFER
  typing from the interface's own declared method signature when modeled; else the sole implementer under the existing
  closed-world closure semantics.
- [ ] **Step 4: Run — pass** (all incl. negatives). CDO gate: record the `compoundReceiver` drop + rate; `genuine_wrong`
  stays 0; **ADJUDICATE EVERY newly-`Resolved`/`Catalog` chain edge** against CDO source (exhaustive). Tighten ratchets with
  a dated note. Deterministic; `sum == unknown`.
- [ ] **Step 5: rustfmt + clippy + (no-CDO) test + commit** — `feat(resolve): resolve Var.Method().X() cross-object
  call-result chains via pure resolve_member type-query, fail-closed (Task 3)`.

---

### Task 4: Leaf tables — Xml framework entries, the RecordRef-family typed-return table, HTTPCONTENT refresh

**Files:** Modify `src/program/resolve/framework_returns.rs`, `member_catalog.rs` (or a sibling module),
`src/program/resolve/receiver.rs` (the RecordRef-family arm); Test `tests/r0-corpus/ws-chain-tables/` + CDO gate.

- [ ] **Step 1: Write failing + negative fixtures.** POSITIVES: (a) `XmlElement.Create(...).AsXmlNode()` +
  `Node.AsXmlElement().GetChildNodes()` + `Child.AsXmlText().Value()` (the real CDO shapes) resolve via new Xml entries;
  (b) `RecRef.KeyIndex(1).FieldIndex(1).Value()` — `KeyIndex→KeyRef`, `FieldIndex→FieldRef`, leaf resolves (Catalog);
  (c) `Content.AsText()` on a typed `HttpContent` receiver resolves via the refreshed HTTPCONTENT set. NEGATIVES
  (→ `Unknown`): un-tabled Xml member; wrong arity/form (a method-entry invoked as a property, a property invoked with
  parens+args, wrong arg count); a same-named member on a non-framework receiver; an unvalidated/omitted entry stays
  declined; **`FieldRef.Value` chain-decline (round-1 I4)** — `FieldRef.Value` returns variant-like field data whose type
  cannot be known without a table-field index: `SourceRecRef.Field(1).Value().SomeMethod()` stays `Unknown` (Value is a
  LEAF — resolvable as a final catalog member, NEVER a chainable typed receiver).
- [ ] **Step 2: Run — fail.**
- [ ] **Step 3: Implement.** (i) Xml entries in `framework_returns.rs` — per-entry MS-Learn provenance, keyed
  `(kind, member_lc, is_method, arity)` (method-vs-property form is part of the key — an entry never fires on the wrong
  form), uncertain OMITTED; (ii) a new typed-return table for the `(RecordRef|FieldRef|KeyRef, member_lc, is_method, arity)
  → ReceiverType` family + the `ReceiverType::{RecordRef,FieldRef,KeyRef}` arm in the compound step (same fail-closed
  mechanism as Framework, distinct family — table-miss → decline). **Type-stable HANDLES only** (round-1 I4):
  `Field/FieldIndex → FieldRef`, `KeyIndex → KeyRef`, `FieldCount/KeyCount → scalar-decline`; **`FieldRef.Value` (and any
  variant-like return) gets NO chainable entry** — leaf catalog membership only, so a chained `.X()` on it declines;
  (iii) extend HTTPCONTENT with the SymbolReference/MS-Learn-verified methods (`AsText`, `AsBlob`, `AsInStream`, `AsJson*`,
  …) — **round-2 critical: new entries must NOT land in a name-only emission path.** If the existing phf-set lookup emits
  `Catalog` on bare name membership, the new entries go into an arity/form-KEYED table (the `framework_returns`-style
  `(family, member_lc, is_method, arity)` shape) with the lookup layer checking form/arity before emission — never extend
  bare-name membership alone. Same rule for the Xml + RecordRef-family leaf entries.
- [ ] **Step 4: Run — pass** (incl. negatives). CDO gate: record the further `compoundReceiver` drop; `genuine_wrong` 0;
  ADJUDICATE every new edge + every table entry hit; tighten ratchets; deterministic; `sum == unknown`.
- [ ] **Step 5: rustfmt + clippy + (no-CDO) test + commit** — `feat(resolve): Xml + RecordRef-family typed-return tables +
  HTTPCONTENT refresh, fail-closed (Task 4)`.

---

### Task 5: Re-measure, adjudicate, tighten ratchets, CHANGELOG + charter memory

**Files:** Modify `tests/program_resolve_harness.rs` (ratchets), `CHANGELOG.md`, charter memory; Test.

- [ ] **Step 1: Full re-measure** (WITH `CDO_WS` + `ENFORCE_CDO_WS=1`, SINGLE tests): FINAL rate + `unknown` + the full
  `unknownByReason` (the `compoundReceiver` bucket should have dropped substantially across Tasks 3–4; Task 1 CDO-neutral).
  `genuine_wrong=0`, `sum == unknown`, `fresh_missing`.
- [ ] **Step 2: Exhaustive-adjudication sign-off** — every newly-`Resolved`/`Catalog` chain/table edge hand-adjudicated
  (count == the bucket drop; none depends on a mislabeled-protected ABI member — now impossible post-Task-1, state it).
  **Adjudicate CHANGED edges too (round-1 M3):** diff the full before/after edge dump — any edge whose TARGET or EVIDENCE
  changed (not just Unknown→Resolved conversions) is part of the adjudication set; T1/T3 can retarget an existing edge.
- [ ] **Step 3: Tighten ratchets to the measured floor** (rate + count DOWN). Keep `genuine_wrong==0` +
  FRESH_WRONG/FRESH_MISSING.
- [ ] **Step 4: Run** — (no CDO) green; (WITH `CDO_WS`) all gates green, deterministic.
- [ ] **Step 5: CHANGELOG (honest, SCOPED)** — T1 protected-ABI (dormant on CDO — fixture-proven; ABI-only workspaces now
  safe); T2 structured ABI returns (param-fingerprint sibling gap noted as follow-up); T3 cross-object chains; T4 tables;
  the MEASURED net delta. DEFERRED stays visible: record-field chains (needs the table-field index), `untrackedReceiver=91`,
  reclassification, ABI param fingerprints.
- [ ] **Step 6: Charter memory** — append the arc + roadmap; update `MEMORY.md` pointer. Commit — `docs(resolve):
  cross-object chains + protected-ABI complete — real-unknown 1.82%→X% (Task 5)`.

---

## Roadmap — beyond this plan

Record-field chains (`Rec."Field".X()` — a table-field type index on `ObjectNode`; `FieldDecl` already parsed, zero
consumers); `untrackedReceiver=91`; honest-taxonomy reclassification (OverloadAmbiguous/MemberNotFound → charter §5
sub-states); ABI param-fingerprint degradation (the `Subtype` drop on parameters); protected `Variables[]` (`"Protected":true`
on dep page/table vars — relevant once var-access modelling exists). North-star (charter §8): the residual becomes provably
dynamic, risk-weighted by centrality.
