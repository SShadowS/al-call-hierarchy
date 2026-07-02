# Applicability-checker fix + ABI param-Subtype fidelity + record-field chains Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

> Status: **v2.1** (round 2: gemini GO, gpt GO-WITH-CHANGES — remaining guardrails folded as the "Round-2 addenda" section:
> IncludeSender tri-state single-source + ABI probe policy + preflight; FP canonicalization; marker at the final selection
> boundary; T4 static var-extraction audit; duplicate-decline logging. Round 1 = both GO-WITH-CHANGES; criticals folded: the `+1` tolerance is CONDITIONAL on
> the publisher's `IncludeSender=true` — tracked at ingestion if absent — in BOTH the wiring and the checker (a blanket +1
> would be synchronized wrongness); the Id-only-subtype collapse sliver closed by including the raw subtype discriminator
> in the param fp AND making PLAIN dispatch decline on the collapse marker (it previously only gated chain type-queries);
> T4 precedence wording fixed (var-first, never fields-first); extension-field folding visibility-scoped-or-fail-closed;
> the sig_fp persistence audit; classify-only (no `is_blob_like` broadening); quoted-only undercoverage documented). Sixth resolution arc (master `f495e24`, CDO real-`unknown` primary 1.75% /
> `unknown=317`: CompoundReceiver 144, UntrackedReceiver 91, OverloadAmbiguous 56, MemberNotFound 25, BuiltinPrecedenceCollision 1;
> `genuine_wrong=0`). Three grounded threads:
> 1. **The `*_applicability` defect (broken gate on master):** `verify_event_subscriber_route`'s strict arity invariant
>    (`differential.rs:571-573`, `sub > publisher → violation`) predates commit `ae35e90`'s Sender-tolerant `+1` wiring
>    (`index.rs:255-306`, `[IntegrationEvent(IncludeSender=true)]` prepends an implicit Sender param). The 200
>    `event_violations` are EXACTLY the +200 legitimately-wired subscribers (`ae35e90`'s own measured delta). Fix = mirror
>    the `+1` in the checker + docs + a regression unit test. Restores the full CDO harness green (126/128 → 128/128).
> 2. **ABI param-Subtype fidelity (soundness, CDO-yield ZERO measured):** `parse_method`'s param mapping takes only the
>    outer `RawTypeDef.name` — same root cause as the fixed return-type gap. Consequence beyond the (already-fail-closed)
>    chain guard: collapsed same-arity ABI overloads resolve CONFIDENTLY via plain dispatch (`Evidence::Abi`, arbitrary
>    survivor) — an UNGUARDED latent false-`Source` class for plain calls. Carrying param Subtype un-collapses genuine
>    overloads (they then correctly decline `OverloadAmbiguous` at dispatch when args can't disambiguate). Frame honestly:
>    defensive plumbing, no CDO metric delta (all CDO deps are EmbeddedSource; 0 collapse-marked routines measured).
> 3. **Record-field chains (the real yield):** `Rec."Field".X()` + bare implicit-Rec `"Field".X()` — measured **44 real CDO
>    sites** (6 in CompoundReceiver, 38 in UntrackedReceiver — the largest single lever left), ALL resolvable via existing
>    catalogs (Blob/Text/Enum) once a table-field type index exists. `FieldDecl` is parsed with zero consumers; add a field
>    surface on `ObjectNode`, fold TableExtension fields (the established `table_extensions_of` pattern), one non-method
>    Member arm, one EnumType-as-chain-base→List entry, and the bare-quoted-field implicit-Rec recognition.

**Goal:** Fix the broken master gate at its root; close the plain-dispatch ABI arbitrary-survivor class; resolve the
record-field chain population — driving CDO real-`unknown` down (~317 → ~275, measured not promised) with **zero
false-`Source`/`Catalog`** (`genuine_wrong` stays 0), every new resolution fixture-proven + exhaustively adjudicated.

**Architecture:** T1 first (a one-line invariant fix + test — unblocks a fully-green CDO harness for every later gate run).
T2 second (ingestion-layer fidelity; includes the ABI table-field Subtype fix T3 consumes). T3 the field index + the
explicit `Rec."Field".X()` arm. T4 the bare implicit-Rec quoted-field capture (the 38). T5 measure/close. All fail-closed;
reuse `classify_type_text`/`parsed_type_to_receiver`/the existing catalogs — zero new type-classification code.

**Tech Stack:** Rust (edition 2024, toolchain 1.96.0). No new dependency. No `engine::l3`/`engine::l2` import in
`src/program/resolve` except `builtins.rs::global_builtins` (grep-guarded).

**Source of truth:** the three grounding reports (this session, file:line + CDO-measured) + master `f495e24` + the charter.

## Key facts (verified on `f495e24`)

**T1 — applicability:**
- `ApplicabilityReport::is_clean()` (`semantic_golden.rs:1209-1216`) folds `event_violations`; both failing tests
  (`fan_out_applicability_zero_violations` `program_resolve_harness.rs:3569`, `route_applicability_zero_violations` :3528-3531)
  fail ONLY on `event_violations=200` — the sole red on master (126/128).
- `verify_event_subscriber_route` (`differential.rs:558-625`; the bound at :571-573 `sub_rid.params_count >
  publisher_params_count → false`) vs the wiring's Sender-tolerant `sub_params <= r.id.params_count + 1`
  (`index.rs:255-306`, commit `ae35e90`). The +200 wired == the 200 violations (same population; the checker flags exactly
  the routes the fix correctly wired). Platform publishers (arity 8) never trip the bound — not the cause; `PublisherKind`
  irrelevant to the checker.
- Fix (v2.1 — CONDITIONAL, never blanket): `include_sender: Option<bool>` on the publisher node (single source of truth;
  ONE shared helper consumed by BOTH `index.rs` wiring and the `differential.rs` checker — no drift); bound =
  `publisher_params + usize::from(include_sender == Some(true))`; unknown → the explicit Step-0 policy (see Task 1). Docs
  cross-referenced; Test-15 panic message prints `event_violations`; a CDO preflight counts
  unknown-IncludeSender-publishers-with-+1-subscribers so any recall movement is attributable.

**T2 — param-Subtype:**
- `parse_method` param mapping (`symbol_reference.rs:680-694`) takes only `RawTypeDef.name`; `RawTypeDef.subtype` is already
  parsed (:193-198) and `reconstruct_return_type_text` (:616-627) + `return_type_subtype_id` (:633-636) already exist —
  generalize/reuse for params. **Decline-path rule differs from returns:** `param_type_fp` is an internal dedup hash with no
  "None = untrustworthy" contract — on the fail-closed decline shapes (Id-only, quote-in-name) FALL BACK TO THE BARE OUTER
  NAME (today's fidelity), never `""`/None (that would regress dedup). Optionally carry `subtype_id` on `AbiParameter`
  (mirroring `return_type_id`; unused today).
- `param_type_fp` (`abi_ingest.rs:32-42`) logic unchanged — output changes via the richer `type_text`. Dedup mechanism
  (`param_sig_key: String::new()` `abi_ingest.rs:341`; `build.rs:334-365`) unchanged — fewer collapses occur. Post-fix,
  un-collapsed genuine overloads hit `resolve_in_object`'s existing >1-candidates → `OverloadAmbiguous` (correct,
  fail-closed). The `abi_overload_collapsed` marker then fires only on true duplicates.
- **Also fix `parse_field` (`symbol_reference.rs:713-718`)** — the same Subtype drop on ABI table FIELDS (an ABI Enum field
  types as bare `"Enum"`); T3's ABI-side field index needs it; same reconstruction helper.
- Blast radius contained (no golden serializes `param_type_fp`/`sig_fp`/`AbiRoutineKey`; `semantic_golden.rs` fixtures are
  source-tier `sig_fp:0`). One live test moves semantically: `ws_cross_object_chain_abi_overload_collapsed_declines`
  (`program_resolve_harness.rs:6919`, fixture N11) — post-fix the two `Get` overloads get distinct `sig_fp` → no collapse →
  2 live candidates → `OverloadAmbiguous` instead of the marker decline. The assert (Unresolved + Unknown(_)) still passes;
  REWRITE its stale doc comment (:6899-6917). Keep a companion fixture that still exercises the collapse marker (two
  IDENTICAL raw entries — a true duplicate). Stale doc comments to fix: `abi_ingest.rs:322-340`, `build.rs:314-333`,
  `resolver.rs:260-282`/:1711-1757 ("ABI params never carry Subtype" → "degraded only on the decline subset").
- **CDO yield: 0 (measured).** All CDO deps are EmbeddedSource; 0 collapse-marked routines. Fixture-proven, like T1 of plan 5.

**T3/T4 — record-field chains:**
- `FieldDecl{number,name,data_type,field_class,is_blob_like}` (`crates/al-syntax/src/ir/decl.rs:56-62`) — zero consumers
  under `src/`. `ObjectNode` (`node_extract.rs:71-91`) has no field surface.
- Design: `pub fields: Vec<FieldNode{name_lc, type_text}>` on `ObjectNode` (mirror `page_controls`), populated in
  `extract_nodes` (source) + `abi_ingest::ingest_abi` (ABI, needs T2's `parse_field` fix). TableExtension fields fold into
  the base's surface via the established `ResolveIndex::table_extensions_of` pattern (`index.rs:505-506`) — a
  `field_in_table(base_lc, field_lc)` lookup checks base + extensions.
- Consumer: `infer_compound_member_receiver` (`receiver.rs:919-1008`) has NO arm for a non-method `Member{object, member}`
  (falls to Unknown at :1007). Add: `!is_method && base_ty == Record{table: Some(tid)}` → `field_in_table` →
  `classify_type_text(&field.type_text)` → `parsed_type_to_receiver` — zero new classification code (`"blob"→Framework(Blob)`,
  `"enum ..."→EnumType`, `"text"/"code"→Framework(Text)` all exist).
- Enum chain base: `ReceiverType::EnumType` already dispatches `Ordinals/Names/AsInteger/FromInteger` via the Enum catalog
  (`resolver.rs:1672-1682`); the multi-level `Rec."eSeal Service".Ordinals().Count()` needs ONE new mapping — EnumType as a
  chain BASE: `Ordinals()`/`Names()` → `Framework(List)` (LIST already has `count`, `member_catalog.rs:180-184`).
- **T4 (the 38):** bare `"Field".X()` inside a Table/TableExtension routine (no dot in `receiver_text` → lands
  `UntrackedReceiver` today). Recognize a QUOTED identifier receiver in Table/TableExt scope as an implicit-Rec field
  (mirror the bare-call Step-3 implicit-Rec precedent) → the same field-index typing. Measured populations: 38 real sites
  (Blob ×~35, one Text[250] `.Trim()`), after excluding 27 report-dataitem-named Record vars (`"Sales Header Filter".GetView()`
  — a quoted DATAITEM name is a Record VAR, not a field). **PRECEDENCE (round-1 critical, one rule):** the existing
  var/param/global/dataitem lookup runs FIRST and WINS (AL scoping — vars shadow fields); the field lookup fires ONLY on a
  var-lookup miss; on any collision/uncertainty → the non-field binding or `Unknown`, NEVER prefer the field. Quoted-only
  is deliberate fail-closed UNDERCOVERAGE (unquoted field receivers `MyBlob.CreateInStream()` are legal AL — deferred,
  documented; T3's explicit `Rec.Field` arm handles both quoted and unquoted since `Rec.` disambiguates).
- Fixtures ground truth: `Rec."Error Message".X()` Blob ×3, `Rec."File Blob"` ×1, `Rec."eSeal Service".Ordinals().Count()`
  (Enum, multi-level), `"File Blob".CreateInStream(...)` bare ×17, etc.

**Gates:** `cdo_full_program_coverage_and_self_reported_metric` (ceilings 0.01751 / 317);
`cdo_l3_semantic_audit_no_fresh_wrong` (`genuine_wrong==0`, `FRESH_MISSING_CEILING=10`, `FRESH_WRONG_CEILING=149`);
`sum(unknownByReason)==unknown`; post-T1 also `fan_out_applicability_zero_violations` + `route_applicability_zero_violations`
(newly green — then GATED for every later task). `CDO_WS="U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud"`, SINGLE tests.

## Round-2 addenda (binding on the tasks; the reviewers' remaining guardrails)

- **T1:** Sender param-TYPE compatibility is NOT validated (arity-only) — document as residual + add the negative fixture
  if the harness can express it. The v1 "Key facts" bullets are superseded by the task steps wherever they conflict.
- **T2 FP canonicalization:** use the project's STABLE fingerprint primitive (never `DefaultHasher`/process-random); fold a
  typed, length-delimited canonical tuple (outer kind + subtype id + raw subtype name + a degradation tag); any residual
  same-key multi-entry group is collapse-marked so collisions OVER-DECLINE, never select.
- **T2 marker placement:** the plain-dispatch collapse-marker guard sits at the FINAL candidate-selection boundary (any
  collapse-marked ABI routine declines before becoming `Source`/`Opaque`, plain call or chain type-query — not one branch).
- **T4 exhaustiveness is a STATIC audit, not a dynamic gate:** audit the var-extraction pipeline's block-scope coverage
  (repeat/while/if/case locals etc.) IN THIS TASK; if gaps exist, T4 does NOT land until they're closed (a missed var
  falling through to a same-named field = false `Source`).
- **T3 duplicate-decline observability:** log the object + duplicate field name on every `None`-by-duplicate; dedupe
  identical source+ABI declarations by provenance before applying the duplicate rule (avoid artificial duplicates).

## Global Constraints

- Rust edition 2024. `rustfmt <file>` per-file — NEVER `cargo fmt`. Stage only named files — NEVER `git add -A`.
  `CHANGELOG.md` per task. CI gates: `cargo clippy --release --all-features -- -D warnings` (NO `--tests`),
  `cargo fmt --check`, `cargo test --workspace` (NO CDO_WS, green).
- **Soundness cardinal.** Field typing only on a UNIQUE field match (base+extensions); var/dataitem-vs-field name collision
  → the AL-correct precedence or fail-closed `Unknown`; scalar/unclassifiable field types decline. T2's un-collapse must
  never SELECT among >1 candidates (inherits `OverloadAmbiguous`).
- **Reuse, don't hand-roll:** `classify_type_text`/`parsed_type_to_receiver`; the shared Subtype reconstruction helper; the
  `table_extensions_of` folding pattern.
- **`genuine_wrong==0` is a regression gate, not proof** — fixtures (positives + fail-closed negatives) + EXHAUSTIVE
  adjudication of every new/changed CDO edge per resolution task.
- Determinism; per-entry provenance on any new catalog entry; ratchets move DOWN with wins, dated notes.
- **Out of scope:** `OverloadAmbiguous`/`MemberNotFound` reclassification; protected `Variables[]`; the remaining
  `untrackedReceiver` non-field residual; cross-object deeper chains beyond the landed arm.

## File / module structure

| File | Task |
|------|------|
| `src/program/resolve/differential.rs` + `semantic_golden.rs` (docs/test) | T1 |
| `src/engine/deps/symbol_reference.rs` + `src/program/abi_ingest.rs` + `build.rs` (docs) | T2 |
| `src/program/node_extract.rs` (FieldNode) + `abi_ingest.rs` (ABI fields) + `index.rs` (field_in_table) + `receiver.rs` (the arm) | T3 |
| `src/program/resolve/receiver.rs` (bare quoted-field recognition) | T4 |
| `tests/r0-corpus/**` + `tests/program_resolve_harness.rs` + `CHANGELOG.md` + charter memory | T1–T5 |

---

### Task 1: Fix the event-applicability checker — mirror the Sender-tolerant `+1` bound

**Files:** Modify `src/program/resolve/differential.rs`, `semantic_golden.rs`; Test unit + the 2 CDO gates.

- [ ] **Step 0: Ground `IncludeSender` availability + set the unknown-policy.** The `+1` is ONLY legal AL when the publisher
  declares `[IntegrationEvent(IncludeSender=true, …)]` — a blanket +1 in wiring AND checker is SYNCHRONIZED WRONGNESS.
  Determine whether the flag is parsed/ingested today (source `[IntegrationEvent]` attribute args on the publisher path;
  the ABI `SymbolReference` event metadata). If NOT tracked → add `include_sender: Option<bool>` at ingestion (ONE field,
  ONE shared helper used by wiring AND checker — tri-state, single source of truth) BEFORE the tolerance work. **Unknown
  policy (round-2):** source-tier publishers always have the attribute → `Some`. For ABI-tier publishers, PROBE real
  SymbolReference event entries first — if the flag is reliably present, use it; if the schema genuinely cannot carry it,
  set an EXPLICIT documented policy (fail-closed no-+1 by default; IF the CDO preflight shows that obliterates a large
  legit +1 population on un-inspectable standard ABI events, escalate the tradeoff in the report rather than silently
  choosing). Emit the preflight diagnostic: count of unknown-IncludeSender publishers with +1-arity subscribers.
- [ ] **Step 1: Write the failing regression units** — hand-built graphs: (a) `IncludeSender=true`, 0-arity publisher +
  1-arity Sender-capturing subscriber → route wired, `event_violations == 0` (fails today: the strict bound flags it);
  (b) NEGATIVE: `IncludeSender=false` (or unknown), 0-arity publisher + 1-arity subscriber → NOT wired (or flagged) — the
  conditional must reject it. Place beside the existing `event_route_*` units (~`semantic_golden.rs:3140/:3198`).
- [ ] **Step 2: Run — fail.**
- [ ] **Step 3: Implement — CONDITIONAL tolerance, both sides.** `index.rs:255-306` (wiring): `sub_params <=
  r.id.params_count + usize::from(publisher_include_sender)`; `differential.rs:571-573` (checker): the same conditional
  bound. Update the doc comments (`differential.rs:543-557` condition 1; `semantic_golden.rs:1172-1177`) to state the
  CONDITIONAL Sender tolerance with a cross-reference so the two can't drift. Fix Test-15's panic message to print
  `event_violations` (observability gap).
- [ ] **Step 4: Run — pass** + (WITH `CDO_WS`, SINGLE) `fan_out_applicability_zero_violations` +
  `route_applicability_zero_violations` now GREEN (event_violations 200→0 — the 200 are IncludeSender-true routes, per the
  `ae35e90` population math); `cdo_full_program_coverage_and_self_reported_metric` + `cdo_l3_semantic_audit_no_fresh_wrong`.
  EXPECTATION: byte-identical 1.75%/317 IF no non-IncludeSender over-wired route exists on CDO; if the conditional wiring
  REMOVES over-wired routes, that is a false-`Source` correction — ADJUDICATE each (don't auto-fail), record the delta.
  Full CDO harness 128/128.
- [ ] **Step 5: rustfmt + clippy + (no-CDO) test + commit** — `fix(resolve): event-applicability checker honors the implicit
  Sender +1 arity tolerance — event_violations 200→0, CDO harness fully green (Task 1)`.

---

### Task 2: ABI param-Subtype fidelity — un-collapse genuine overloads (fail-closed, CDO-neutral)

**Files:** Modify `src/engine/deps/symbol_reference.rs`, `src/program/abi_ingest.rs`, doc comments in `build.rs`/`resolver.rs`;
Test unit + probe `.app` + CDO gates.

- [ ] **Step 1: Write failing tests** — (a) a param with `Subtype{Name:"Dep A",Id:..}` yields `type_text = Codeunit "Dep A"`;
  (b) decline shapes (Id-only, quote-in-name) FALL BACK to the bare outer name for the TEXT — but the param FP additionally
  folds the RAW SUBTYPE DISCRIMINATOR (the Id, or a hash of the raw name) so **two DIFFERENT Id-only subtypes do NOT
  collapse** (the round-1 critical sliver: `DoIt(Codeunit 10)` vs `DoIt(Codeunit 20)` must stay distinct → dispatch declines
  `OverloadAmbiguous`, never an arbitrary survivor); (c) `parse_field` same treatment (an ABI Enum field carries `Enum "X"`);
  (d) the N11 probe pair now ingests as TWO distinct nodes, un-collapsed, and plain dispatch declines `OverloadAmbiguous`;
  (e) a TRUE-duplicate probe pair (identical raw entries incl. discriminators) still collapse-marks; (f) **PLAIN-DISPATCH
  MARKER GUARD (round-1 critical):** plain `resolve_in_object` dispatch on a collapse-MARKED SymbolOnly candidate declines
  (the marker previously only gated chain type-queries — a marked survivor must never resolve confidently via plain
  dispatch either); (g) NEUTRALITY: CDO byte-identical (0 marked on CDO).
- [ ] **Step 2: Run — fail.**
- [ ] **Step 3: Implement** — generalize the reconstruction helper for params + fields (bare-fallback TEXT rule + the
  discriminator-bearing FP rule — text and fp serve different contracts); `parse_method` + `parse_field` carry the
  Subtype-qualified text (+ `subtype_id` on `AbiParameter`, now REQUIRED for the fp fold); extend the plain-dispatch path
  (`resolve_in_object`'s SymbolOnly selection) to decline collapse-marked candidates (defense in depth beside the chain
  guard); **sig_fp persistence audit** — grep for any serialization/persistence of `RoutineNodeId`/`AbiRoutineKey`/`sig_fp`/
  `param_type_fp` (caches, incremental artifacts, CI baselines); if none (expected), DOCUMENT that ABI node identity is not
  stable across fidelity changes; if any, version-bump it. Rewrite the stale doc comments (`abi_ingest.rs:322-340`,
  `build.rs:314-333`, `resolver.rs:260-282`/:1711-1757 — the marker now fires on exact duplicates AND unsafe
  indistinguishable degraded overloads, not "only true duplicates") and the N11 test's doc (`program_resolve_harness.rs:6899-6917`).
- [ ] **Step 4: Run — pass** + CDO byte-identical (1.75%/317; 0 collapse-marked before AND after — the fix is dormant on
  CDO by construction; if anything moves, STOP) + the 2 applicability gates stay green.
- [ ] **Step 5: rustfmt + clippy + (no-CDO) test + commit** — `feat(resolve): ABI param/field Subtype fidelity — genuine
  overloads un-collapse and decline honestly; bare-fallback on decline shapes (Task 2)`.

---

### Task 3: Table-field type index + the `Rec."Field".X()` arm + EnumType chain base

**Files:** Modify `src/program/node_extract.rs`, `abi_ingest.rs`, `src/program/resolve/index.rs`, `receiver.rs`,
`framework_returns.rs` (or the Enum catalog site); Test `tests/r0-corpus/ws-record-field-chain/` + CDO gates.

- [ ] **Step 1: Write failing + negative fixtures.** POSITIVES: (a) `Rec."Error Msg Blob".CreateInStream(S)` (Blob field →
  Framework(Blob) → catalog member resolves); (b) `Rec."Doc Status".Ordinals().Count()` (Enum field → EnumType →
  `Ordinals()` → Framework(List) → `Count()` resolves — the multi-level chain); (c) a TableExtension-declared field on the
  base table resolves the same way (folding); (d) an ABI-tier table's field (needs T2's parse_field) types correctly.
  NEGATIVES (→ `Unknown`): unknown field name; a field whose type is scalar (`Integer`) — member call on it declines;
  DUPLICATE field name across base+extensions (fail closed); a Page (non-Record) receiver with a quoted member; a
  var/dataitem named like a field — the non-field binding wins or declines (NO field mis-typing).
- [ ] **Step 2: Run — fail.**
- [ ] **Step 3: Implement** — `FieldNode{name_lc, type_text}` on `ObjectNode` (populated: source `extract_nodes` from
  `FieldDecl`; ABI `ingest_abi` from the T2-fixed `parse_field`). **Classify strictly via `classify_type_text` on the
  declared type text — do NOT broaden via `is_blob_like`** (Media/MediaSet etc. have no Blob catalog; mis-mapping = false
  `Catalog`). `ResolveIndex::field_in_table(from_object, base, field_lc)` checking base + extension fields
  **VISIBILITY-SCOPED**: include only extensions visible from the referencing object (same app or dependency closure — the
  same closure discipline the resolver uses everywhere); an out-of-closure extension's field must NOT resolve (fail closed);
  UNIQUE match or None (log/count duplicate-field declines for measurement). The non-method Member arm in
  `infer_compound_member_receiver` (`!is_method` + `Record{table: Some}` → field → `classify_type_text` →
  `parsed_type_to_receiver`), handling BOTH quoted and unquoted member names (`Rec.` disambiguates). The
  EnumType-as-chain-base entry (`Ordinals`/`Names` → Framework(List), provenanced). All declines → `Unknown`.
- [ ] **Step 4: Run — pass.** CDO gates: `compoundReceiver` should drop by ~6; `genuine_wrong=0`; applicability green;
  EXHAUSTIVE adjudication of every new/changed edge; ratchets tightened, dated.
- [ ] **Step 5: rustfmt + clippy + (no-CDO) test + commit** — `feat(resolve): table-field type index + Rec."Field".X()
  record-field chains + EnumType chain base, fail-closed (Task 3)`.

---

### Task 4: Bare implicit-Rec quoted-field receivers (`"Field".X()` in Table/TableExt scope)

**Files:** Modify `src/program/resolve/receiver.rs`; Test fixtures + CDO gates.

- [ ] **Step 1: Write failing + negative fixtures.** POSITIVES: (a) inside a Table's procedure, `"File Blob".CreateInStream(S)`
  → the implicit-Rec field types Framework(Blob) → resolves; (b) same inside a TableExtension procedure (the base+own field
  surface). NEGATIVES (→ `Unknown` or the correct non-field binding): (c) a quoted RECORD VAR / report-dataitem name used as
  a receiver (`"Sales Header Filter".GetView()`) — the VAR binding wins (AL scoping: locals/params/globals/dataitems shadow
  fields) — assert it still resolves AS A VAR, never as a field; (d) a quoted name matching BOTH a var and a field → the var
  wins (precedence pinned); (e) quoted-field receiver in a NON-Table/TableExt object (no implicit Rec) → `Unknown`; (f) an
  unknown quoted name → `Unknown`.
- [ ] **Step 2: Run — fail.**
- [ ] **Step 3: Implement** — in `infer_receiver_type`'s bare-identifier path: AFTER the existing var/param/global/dataitem
  lookup (which must keep winning — AL scoping; PRE-REQ: confirm the var-extraction surface is exhaustive for the scopes
  involved — a MISSED var declaration falling through to a same-named field would be a false-`Source` mis-type, so any
  scope the var extractor doesn't cover must DECLINE rather than fall through), if the receiver is a QUOTED identifier and
  `from_object` is a Table/TableExtension (the implicit-Rec scope, mirroring the bare-call Step-3 precedent incl. its
  `with_state` gating), look up the field in the implicit-Rec table surface (`field_in_table`, visibility-scoped) → type by
  the field. Decline on collision/ambiguity/non-Table scope. Quoted-only = documented deliberate undercoverage (unquoted →
  roadmap).
- [ ] **Step 4: Run — pass.** CDO: `untrackedReceiver` should drop substantially (~38 sites measured); `genuine_wrong=0`;
  applicability green; EXHAUSTIVE adjudication (every new/changed edge — esp. verify NO dataitem/var got mis-typed as a
  field); ratchets tightened, dated.
- [ ] **Step 5: rustfmt + clippy + (no-CDO) test + commit** — `feat(resolve): bare implicit-Rec quoted-field receivers in
  Table scope, var-precedence fail-closed (Task 4)`.

---

### Task 5: Re-measure, adjudicate, ratchets, CHANGELOG + charter memory

- [ ] **Step 1: Full re-measure** (WITH `CDO_WS` + `ENFORCE_CDO_WS=1`, SINGLE): both metric gates + BOTH applicability gates
  + full breakdown; `sum==unknown`; `genuine_wrong=0`.
- [ ] **Step 2: Adjudication sign-off** — every T3/T4 new/changed edge == the bucket drops; none mis-typed a var/dataitem.
- [ ] **Step 3: Ratchets to the measured floor** (rate + counts DOWN; dated).
- [ ] **Step 4: Run all gates.**
- [ ] **Step 5: CHANGELOG (honest, scoped)** — T1 the gate fix (root-caused to `ae35e90`); T2 dormant-on-CDO plumbing
  (fixture-proven); T3/T4 the measured drops. DEFERRED visible: untrackedReceiver residual, reclassification, protected
  Variables[], deeper chains.
- [ ] **Step 6: Charter memory + MEMORY.md pointer. Commit** — `docs(resolve): applicability fix + param-Subtype +
  record-field chains complete — real-unknown 1.75%→X% (Task 5)`.

---

## Roadmap — beyond this plan

The honest-taxonomy reclassification (OverloadAmbiguous 56 / MemberNotFound 25 → charter §5 sub-states); the remaining
`untrackedReceiver` non-field residual; protected `Variables[]` (once var-access modelling exists); deeper cross-object
chains; risk-weighted centrality reporting (charter §8).
