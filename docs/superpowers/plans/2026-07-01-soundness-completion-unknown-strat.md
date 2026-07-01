# Fail-closed soundness completion + honest Unknown stratification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

> Status: **v2.1** — round 2 both GO-WITH-CHANGES→GO (no v3). Round-2 tightenings: (a) `Evidence::Unknown(UnknownReason)`
> would churn the anonymized semantic goldens' serde → add `Evidence::kind()` and serialize/compare the KIND at the
> golden/audit boundary (reason lives ONLY in the aldump breakdown; goldens byte-identical, NO regen); (b) `object_extends`
> is DIRECT + kind-compatible (not transitive, not reverse, never peer); (c) no AL compiler in CI → check in a
> `COMPILER_PROOF.md` artifact (required for the protected self/peer/extension cases); (d) reworded Task 4's stale "346
> unchanged"; (e) `UnknownReason::as_str()` stable spelling; (f) `ObsoleteState=Removed` added to the roadmap as a latent
> false-`Source` vector to check.
>
> v2 — round 1 gpt-5.5 GO-WITH-CHANGES + gemini-3.1-pro GO-WITH-CHANGES. Convergent fixes folded in:
> (1) **Protected visibility by resolved OBJECT IDENTITY, generalized across extension kinds** (not `kind==TableExtension`
> + LC-name), so a PageExtension/ReportExtension can reach its base object's protected members AND peer-extensions can
> NEVER see each other's protected (the sibling-bleed false-`Source`). (2) **`Evidence::Unknown(UnknownReason)` payload**
> (not `Option<UnknownReason>` on `Route`) — exhaustive by construction, no per-edge memory bloat, required-reason
> constructors, `BTreeMap`/stable ordering, a code-level "every Unknown has a reason" invariant. (3) **Only Task 3 is
> count-preserving; Task 1/2 MAY raise the real-`unknown` count** (they turn false-`Source`→honest-`Unknown` — soundness
> beats the metric; the ratchet must not block a soundness fix). (4) Expanded **compiler-backed fixture matrix**
> (same-app-protected-non-extension, protected-self, protected-in-TableExt self-vs-peer, PageExt-base-protected, internal
> same/cross), each with the AL-compiler semantics stated. (5) Cross-app `internal` (friend-app `internalsVisibleTo`) is
> out of scope → fail-closed `Unknown`, documented. (6) TDD Step-2 asserts the EXACT pre-fix wrong route.
>
> v1 (DRAFT). The next plan after the two merged resolution arcs (master `fd34b91`,
> CDO real-`unknown` 6.46%→1.91%). **Deliberately measure-precisely-first**, not a metric chase — grounding
> (this session) proved the residual 346 Unknown edges can't be targeted without diagnostic reason-tagging, and
> that no cheap taxonomy-reclassification lever exists (dynamics are already `HonestDynamic`, 42==42 vs L3).
> Two strands:
> 1. **Complete the fail-closed thesis** — close the last two reviewer-flagged latent false-`Source` vectors:
>    cross-app `Access::Protected` unfiltered + same-app `local` treated as app-scoped (both in one helper), and
>    the `resolve_object_name_lc` numeric-vs-quoted shape-loss (symmetric to the landed I1 `Record` fix).
> 2. **Stratify the residual** — the fresh engine emits a BARE `Evidence::Unknown` (no reason) from 13 structurally
>    distinct decline sites; add a fresh-native diagnostic `UnknownReason` + a stratified `aldump` breakdown so the
>    346 are precisely characterized (charter §8 stratified reporting). This is the prerequisite for the NEXT plan's
>    targeted burndown + the honest-taxonomy reclassification decision — both made WITH data, not guessed here.

**Goal:** Close the two remaining latent false-`Source` vectors (fail-closed — Task 1/2 MAY raise the real-`unknown`
count as a soundness correction, and that is CORRECT), and precisely characterize the 346 CDO real-`unknown` edges by
adding fresh-native reason-tagging (Task 3 is diagnostic-only — count + classification unchanged), with `genuine_wrong`
staying 0. **Soundness beats the metric: eliminating a false `Source` takes priority over the real-`unknown` rate.**

**Architecture:** Task 1 restructures the one visibility helper (`object_has_visible_member_candidate`) to be
caller-identity-aware — resolving BOTH the `Protected` and same-app-`local` gaps in a single access-branch rewrite.
Task 2 mirrors the I1 `ParsedType::Record` shape-preservation for `ParsedType::Object`. Task 3 adds a fresh-native
`UnknownReason` (NOT `engine::l3`'s — the grep-guard invariant holds) tagged at the 13 Unknown-route sites, surfaced
via a stratified stat. Task 4 re-measures + documents the precise breakdown as the next-plan roadmap. NO reclassification,
NO new resolution logic — this plan measures and hardens; it does not burn down.

**Tech Stack:** Rust (edition 2024, toolchain 1.96.0). No new dependency. No `engine::l3`/`engine::l2` import in
`src/program/resolve` except `builtins.rs::global_builtins` (grep-guarded; the new `UnknownReason` is fresh-native).

**Source of truth:** the two grounding reports (this session, file:line-verified) + master `fd34b91` + the charter
(§5 taxonomy incl. the `ambiguous`/`memberNotFound` diagnostic sub-states, §8 stratified + risk-weighted reporting).

## Key facts grounding this plan (all file:line verified on master `fd34b91`)

**Soundness (Tasks 1–2):**
- `object_has_visible_member_candidate` (`src/program/resolve/resolver.rs:331-360`): after `object_has_member_candidate`
  proves a name+arity match, the **same-app branch returns `true` UNCONDITIONALLY** (`:343`, `obj_id.app == from_app`)
  — no self-check, no access filter; the cross-app branch (`:350-359`) filters only `Local`/`Internal` (+ `None`
  fail-closed), NOT `Protected`. Sole caller: `resolve_in_table_scope` (`:451-459`, has `from_object: &ObjectNode`
  with `id` + `extends_target`; `scope = [table_id] + index.table_extensions_of(table_name_lc)`).
- The access model is complete: `Access` enum with `Protected` (`src/program/node_extract.rs:14-20`);
  `Access::from_modifier` (`:22-30`); `RoutineNode.access` (`:98`, populated `:303`, source-tier only);
  `lookup_routine_access` (`resolver.rs:301-307`) returns `Option<Access>`. **ABI/SymbolOnly tier hardcodes
  `Access::Public` (`abi_ingest.rs:283`) and drops local/internal at ingestion (`:263`)** — so `Protected`/`local`
  gaps are scoped to SOURCE-tier cross-app/same-app objects only.
- AL semantics: `local` = callable only within the DECLARING object (object-scoped, NOT app-scoped); `internal` =
  app-scoped; `protected` = visible to the declaring object AND its extensions. Existing test
  `resolve_member_record_cross_app_extension_local_method_excluded` (`resolver.rs:3559-3610`) covers cross-app `Local`;
  NO same-app-different-object `Local` test, NO `Protected` test.
- `resolve_object_name_lc` (`src/program/resolve/receiver.rs:859-880`): does `name.trim().parse::<i64>()` on an
  ALREADY-normalized string → `Codeunit "80"` and `Codeunit 80` both resolve by id 80 (wrong for an object literally
  named `"80"`). Shape lost by `parse_object_kind_type` (`:883-888`, unquotes unconditionally into
  `ParsedType::Object{kind, name:String}` `:235-237`). The I1 fix ALREADY landed the pattern to mirror:
  `ParsedType::Record{table_ref: ObjectRef}` (`:224-234`), `ObjectRef {Name{raw,normalized_lc}|Id(i64)}`
  (`node_extract.rs:42-48`), routed through the kind-generic `index.resolve_object_ref` (`index.rs:414`, takes any
  `ObjectKind`). Caller `parsed_type_to_receiver` Object arm (`:835-844`, has `from_object`) currently forces a
  redundant second by-name lookup (`resolver.rs:1082`) that setting `id: Some` up front removes.
- **Record-builtin-collision guard (Item 4) is NOT a bug — do NOT "fix" it.** `is_bare_builtin_or_page_intrinsic`
  (`resolver.rs:519-525`) checks `Global` + `Framework(PageInstance)` only; `resolve_member`'s Record arm lets a
  source candidate shadow the Record catalog without a collision check. Grounding proved (test
  `resolve_member_record_source_proc_shadows_same_named_builtin` `resolver.rs:3167-3224`, the 42-CDO-instance corpus)
  that Source-shadows-Catalog for a `Record` receiver is CORRECT AL precedence; adding a `Record` guard would REGRESS
  the beyond-1B.3b Task 1 fixes into false-`Unknown`. Scope: an explanatory comment ONLY.

**Unknown stratification (Task 3):**
- `Evidence` (`src/program/resolve/edge.rs:117-124`) is a BARE unit enum `{Source, Abi, Catalog, Opaque, Unknown}` —
  no payload, no reason. `grep UnknownReason src/program/` = zero hits (it exists only in `engine::l3::call_resolver`,
  never imported — grep-guarded). `aldump --program-call-graph-stats` prints only the aggregate histogram.
- The 346 originate from ~13 distinct `member_unknown_route()`/`unknown_route()` call sites in `resolver.rs`/`full.rs`,
  each a distinct honest-decline cause (grounding enumerated): `ReceiverType::Unknown` (compound/untracked receiver);
  `CalleeShape::Unknown` (unclassifiable expr); overload ambiguity `>1` candidates (`:466`, `:1240`); table-proc↔builtin
  precedence collision (`:627-628`); `with`-scope guard-decline; Codeunit-`TableNo` bare-Rec (strict-kind excluded);
  Report/ReportExtension implicit Rec (excluded); `Access::Protected` decline (Task 1); `RecordRef`/`FieldRef`/`KeyRef`/
  `Framework`/`EnumType` catalog-miss (`:1003-1029`, `:1256-1265`); Object-receiver out-of-closure / no-identity
  (`:1084-1087`); member-not-found (object resolved, method/arity absent).
- `classify_obligation` (`edge.rs:250`); `ObligationOutcome {Resolved, ConditionalResolved, HonestDynamic, HonestEmpty,
  Unknown}` — no `Ambiguous`/`MemberNotFound` variant (L3 has them as non-failure buckets: `ambiguous=55`,
  `memberNotFound=81` on the narrower L3 population). **This plan does NOT add those variants or reclassify** — it only
  TAGS the reason so the next plan can decide, with data, which reasons are honest sub-states vs true failures.
- CDO baseline (measured this session): `primaryScoped total=18104, resolvedSource=8204, resolvedCatalog=5615,
  resolvedAbiExternal=4, conditionalResolved=17, honestDynamic=42, honestEmpty=3876, unknown=346` (1.91%).

**Metric gates:** `cdo_full_program_coverage_and_self_reported_metric` (primary `<= 0.021`, measured 1.91%);
`cdo_l3_semantic_audit_no_fresh_wrong` (`genuine_wrong == 0`, `FRESH_MISSING_CEILING = 10`, `FRESH_WRONG_CEILING = 149`).
`CDO_WS="U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud"`.

## Global Constraints

- Rust edition 2024; toolchain 1.96.0. `rustfmt <file>` per-file — NEVER `cargo fmt`. Stage only named files — NEVER
  `git add -A`. Update `CHANGELOG.md` per task.
- CI gates: `cargo clippy --release --all-features -- -D warnings` (NO `--tests`), `cargo fmt --check`,
  `cargo test --workspace` (NO `CDO_WS`, green). New fixtures under `tests/r0-corpus/`.
- **Soundness is the cardinal rule.** The Task-1/2 fixes only make resolution MORE fail-closed (decline where AL can't
  see the member / where the object-ref shape is ambiguous). `genuine_wrong == 0` MUST hold after every task; the
  soundness fixes may CORRECT a few over-resolutions (like beyond-1B.3b Task 2's +6) — confirm any CDO movement is a
  `fresh_extra`→`matches` correction, never a `matches`→`fresh_missing` over-decline.
- **Count discipline (split — gpt/gemini round-1):** Task 3 (diagnostic tagging) is COUNT- AND CLASSIFICATION-preserving
  (assert the `ObligationOutcome` histogram is byte-identical before/after — the reason is an annotation, nothing moves).
  Task 1/2 (soundness fixes) MAY raise the real-`unknown` count where they turn a false-`Source` into honest `Unknown`
  — that is the CORRECT outcome; the coverage ratchet must NOT block it. **No reclassification** of
  `ambiguous`/`memberNotFound`/`external` out of `Unknown` here — that is the NEXT plan's decision, made with this plan's
  per-reason data + its own external review (blind reclassification is metric-laundering; we refuse it without the data).
- **Fresh-native diagnostic only.** The new `UnknownReason` is defined in `src/program/resolve` (NOT imported from
  `engine::l3`); the grep-guard invariant holds. It is a DIAGNOSTIC tag, not an oracle input.
- Determinism (charter §C8). No `engine::l3`/`engine::l2` import in `src/program/resolve` except `builtins.rs::global_builtins`.
- **Out of scope (the data-driven NEXT plan):** reclassifying honest sub-states (`ambiguous`/`memberNotFound`/out-of-closure
  `external`) out of real-`unknown`; the actual burndown levers (Codeunit-`TableNo` bare-Rec, `with`-scope RESOLUTION,
  overload arg-type DISPATCH, Report implicit-`Rec`, catalog completeness). This plan MEASURES + HARDENS; it does not burn down.

## File / module structure

| File | Responsibility |
|------|----------------|
| `src/program/resolve/resolver.rs` (modify) | Task 1: caller-identity-aware `object_has_visible_member_candidate` (Protected + same-app-local) + Item-4 comment. Task 3: `UnknownReason` tagging at the 13 decline sites. |
| `src/program/resolve/receiver.rs` (modify) | Task 2: `ParsedType::Object{object_ref}` shape-preservation + route through `resolve_object_ref`. |
| `src/program/resolve/edge.rs` (modify) | Task 3: fresh-native `UnknownReason` enum + carry it on the Unknown route/edge. |
| `src/bin/aldump.rs` (modify) | Task 3: stratified Unknown-reason breakdown in `--program-call-graph-stats` (or a new flag). |
| `tests/r0-corpus/**` + `tests/program_resolve_harness.rs` (create/modify) | Tasks 1/2/3: fixtures + the stratified-breakdown assertion + CDO gates. |
| `CHANGELOG.md` + charter memory (modify) | Task 4. |

---

### Task 1: Caller-identity-aware visibility — close cross-app `Protected` + same-app `local` (one helper)

**Files:** Modify `src/program/resolve/resolver.rs` (`object_has_visible_member_candidate` + its one call site + the
Item-4 comment); Test `tests/r0-corpus/ws-visibility-local-protected/` + no-CDO harness + CDO gate.

- [ ] **Step 1: Write failing fixtures — the access matrix** (each fixture's AL semantics stated; the positive cases
  compile in real AL, the negatives are AL access errors — `genuine_wrong==0` is a regression backstop, NOT the proof).
  `tests/r0-corpus/ws-visibility-local-protected/`. **Because there is no AL compiler in CI (gpt round-2), the `.al`
  fixtures + resolver assertions are regression coverage, not compiler PROOF — so check in
  `tests/r0-corpus/ws-visibility-local-protected/COMPILER_PROOF.md`** recording, per case, the AL compiler/extension +
  runtime version, whether it compiled (positives) or the exact access-violation diagnostic (negatives), and who/when
  verified it (REQUIRED for the protected self/peer/extension cases — those are exactly where an AL-semantics
  misunderstanding would mint a false `Source`). An optional env-gated (`ALC_EXE`) verifier script is a plus, not mandatory.
  - **`local`:** (a) SELF — an object's OWN `local procedure`, self-call → `Source`. (b) same-app DIFFERENT object — `Table
    Foo` (no `DoWork`) + `TableExtension FooExtB extends Foo` declaring `local procedure DoWork()`; Codeunit `CallerA`
    (same app, different object) `var R: Record Foo; R.DoWork()` → honest `Unknown` (today: false `Source`). (c)
    TableExtension `local` self-call → `Source`; (d) PEER-extension: `FooExtA` declares `local`, `FooExtB` (sibling)
    calls it → honest `Unknown`. (e) cross-app `local` → `Unknown` (existing behavior; guard).
  - **`protected`:** (f) SELF — declaring object calls its OWN `protected` → `Source`. (g) **same-app NON-extension** —
    `Table Bar` with `protected procedure P()`; same-app `Codeunit`/`Page` (SourceTable=Bar) calls `P()` → honest
    `Unknown` (NOT an extension of Bar → invisible). (h) cross-app NON-extension (page SourceTable = dep `Bar`) → `Unknown`.
    (i) **valid extension → base protected**: a `TableExtension` on `Bar` calling `Bar`'s `protected P()` → `Source`;
    and a **`PageExtension` on a base Page** calling the base Page's `protected` proc → `Source` (generalized extension
    kinds, NOT just TableExtension). (j) **PEER-extension protected bleed**: `TableExtension ExtA extends Bar` declares
    `protected P()`; `TableExtension ExtB extends Bar` calls `P()` → honest `Unknown` (ExtB extends Bar, NOT ExtA — the
    sibling-bleed guard, the biggest latent false-`Source`).
  - **`internal`:** (k) same-app → `Source`; (l) cross-app → `Unknown` (fail-closed; `internalsVisibleTo`/friend-app is
    OUT OF SCOPE — documented; this is a false-`Unknown`/recall cost, not the cardinal sin).
- [ ] **Step 2: Run — fail, asserting the EXACT pre-fix wrong route** — (b) currently routes `Source` to `FooExtB.DoWork`;
  (g)/(h) currently route `Source` to `Bar.P`; (j) currently routes `Source` to `ExtA.P`. Assert those exact wrong routes
  now, the corrected honest `Unknown` after (do not merely assert "fails").
- [ ] **Step 3: Implement** — widen `object_has_visible_member_candidate` to take the caller's `ObjectNodeId` (thread
  `from_object.id` from `resolve_in_table_scope` `:451-459`, which has `from_object`). Restructure so NO branch blanket-returns
  `true` for a same-app non-self object; per candidate routine's `Access` (using RESOLVED OBJECT IDENTITY, never LC-name
  comparison — gpt/gemini round-1):
  - `Public` → visible.
  - `Local` → visible ONLY if `candidate.object == from_object.id` (self; `ObjectNodeId` equality).
  - `Internal` → visible ONLY if `candidate.object.app == from_object.id.app` (same app; friend-app `internalsVisibleTo`
    is OUT OF SCOPE → cross-app internal fails closed to `Unknown`, documented).
  - `Protected` → visible ONLY if `candidate.object == from_object.id` (self) OR `from_object` DIRECTLY extends the
    DECLARING object `candidate.object`. `index.object_extends(from, target) -> bool` (identity-based) must be **DIRECT +
    KIND-COMPATIBLE** (gpt round-2): `from.kind.is_extension_kind()` AND `from.kind.extension_base_kind() == Some(target.kind)`
    (a TableExtension extends a Table, PageExtension a Page, …) AND `from`'s `extends_target` RESOLVES (via `resolve_object_ref`,
    identity — NOT LC-name) to `target`. **NOT transitive** (extension-of-extension is not an AL thing — do not model it),
    **NOT reverse** (a base object does NOT see its extension's protected members — the base does not extend the extension),
    **NEVER peer** (ExtB extends the BASE, not ExtA → `resolve_object_ref(ExtB.extends_target) != ExtA` → declines). Add
    `ObjectKind::is_extension_kind()`/`extension_base_kind()` if absent.
  - `None` (access lookup miss) → fail closed (not visible).
  `SymbolOnly` tier → visible (ABI is always `Public`, `abi_ingest.rs:283`). Keep `object_has_member_candidate` (the raw
  counter) unchanged. Add the Item-4 explanatory comment at `is_bare_builtin_or_page_intrinsic` (`:519-525`) + the
  `resolve_member` Record arm: `Record`/`RecordRef` are deliberately NOT collision-guarded — same-receiver
  Source-shadows-Catalog is corpus-validated correct AL precedence (test `resolve_member_record_source_proc_shadows_same_named_builtin`),
  and guarding would regress beyond-1B.3b Task 1.
- [ ] **Step 4: Run — pass** (all fixtures). Then (WITH `CDO_WS`, SINGLE tests) `cdo_l3_semantic_audit_no_fresh_wrong`
  (`genuine_wrong` stays 0) + `cdo_full_program_coverage_and_self_reported_metric`. **The real-`unknown` count MAY RISE
  here** (a `local`/`protected` false-`Source` on CDO correctly becomes honest `Unknown`) — that is a soundness CORRECTION,
  NOT a regression; record any movement + confirm it corresponds to a declined false-`Source` (spot-check the newly-`Unknown`
  site is genuinely an inaccessible member). If the ratchet trips, RAISE it with the justification (soundness > coverage).
  Likely small since these are source-tier cross-app / same-app-cross-object `local`/`protected` cases, rare on CDO.
- [ ] **Step 5: rustfmt + clippy + (no-CDO) test + commit** — `fix(resolve): caller-identity-aware member visibility —
  same-app local is object-scoped + cross-app Protected excluded unless extending; Item-4 comment`.

---

### Task 1.5: Apply the visibility filter to `resolve_bare` Step 2 (extension-base bare calls)

**Inserted after Task 1** (Task 1 review recommendation). `resolve_bare`'s Step 2 ("extension base",
`resolver.rs:606-618`) resolves a bare call against the caller's extended BASE object via `resolve_in_object`
(`resolver.rs:195-235`) — which does ZERO access filtering. So a bare call from a `*Extension` object to a base-object
member currently emits `Source` regardless of the member's access. The reviewer confirmed: `Protected` is INCIDENTALLY
safe here (Step 2's caller is always a direct extension of the base → self-or-extends trivially holds), but **`local`**
(a base object's own-object-scoped member reachable by a bare call from ANY of its extensions) and **cross-app `internal`**
(the extension in a different app than the base) are genuine false-`Source` exposures — pre-existing, in the same
fail-closed class this plan closes. **SOUNDNESS BEATS THE METRIC** (count may rise).

**Files:** Modify `src/program/resolve/resolver.rs` (Step-2 site + optionally `resolve_in_object` or a wrapper); Test
`tests/r0-corpus/ws-bare-extbase-visibility/` + no-CDO harness + CDO gate.

- [ ] **Step 1: Write failing + control fixtures** — (a) `TableExtension ExtA extends Base` bare-calls a `local procedure
  L()` declared on `Base` → honest `Unknown` (today: false `Source` to `Base.L`). (b) CONTROL: `ExtA` bare-calls a
  `procedure Pub()` (public) on `Base` → resolves `Source` (unchanged — Step 2 must still work for public). (c) cross-app
  `internal`: `ExtA` (app X) bare-calls an `internal procedure I()` on `Base` (app Y, X≠Y) → honest `Unknown`. (d) CONTROL:
  `ExtA` bare-calls a `protected procedure P()` on `Base` → resolves `Source` (extension DOES see base protected — confirm
  the incidentally-safe path stays correct). Also a `PageExtension`→base-Page variant of (a)/(b). Assert the EXACT pre-fix
  wrong route for (a)/(c).
- [ ] **Step 2: Run — fail** ((a) `Source` to `Base.L`; (c) `Source` to `Base.I`).
- [ ] **Step 3: Implement** — at `resolve_bare`'s Step-2 site, apply the SAME caller-identity-aware access check Task 1
  added (`object_has_visible_member_candidate(base_id, …, from_object.id)` — the extension is `from_object`, the base is
  the candidate object): a base member is visible to the bare-calling extension per the Task-1 rule (`Local`→base-self only
  = NOT visible to the extension; `Internal`→same-app; `Protected`→extension-of-base = visible; `Public`→visible). Prefer
  reusing the Task-1 helper over duplicating the access logic; do NOT broadly add filtering to `resolve_in_object`'s OTHER
  callers (bare Step 1 own-object, member Object/SelfObject) unless a fixture proves they need it — keep the change scoped
  to the Step-2 extension-base path. (Minor cleanup while here: switch `object_extends`'s O(n) `graph.objects.iter().find`
  to the sorted `binary_search_by` used by `lookup_routine_access`, `resolver.rs:241-247` — house-style consistency.)
- [ ] **Step 4: Run — pass** (all incl. controls). Then (WITH `CDO_WS`, SINGLE tests) `cdo_l3_semantic_audit_no_fresh_wrong`
  (`genuine_wrong` stays 0) + `cdo_full_program_coverage_and_self_reported_metric` — the count MAY rise (a Step-2 `local`/
  cross-app-`internal` false-`Source` correctly becomes `Unknown`); record + spot-check the correction; raise the ratchet
  with justification if it trips (soundness > coverage).
- [ ] **Step 5: rustfmt + clippy + (no-CDO) test + commit** — `fix(resolve): access-filter resolve_bare Step 2 extension-base
  bare calls — base local/cross-app-internal not visible to the extension (Task 1.5)`.

---

### Task 2: `resolve_object_name_lc` shape-preservation (mirror the I1 `Record` fix)

**Files:** Modify `src/program/resolve/receiver.rs`; Test (unit + a fixture). **Dormant on CDO like I1 — the proof is
the synthetic fixture + genuine_wrong=0.**

- [ ] **Step 1: Write failing tests** — unit: `var C: Codeunit 80` (numeric) → resolves by id 80; `var C: Codeunit "80"`
  (a codeunit literally named `80`) → resolves by NAME `"80"` even when id 80 exists (today both resolve by id 80 — the
  shape-loss bug; assert the exact pre-fix wrong resolution). Cover the kinds `resolve_object_name_lc` serves
  (Codeunit/Page/Report/Query/XmlPort). PLUS an end-to-end CALL-GRAPH fixture (`tests/r0-corpus/ws-object-name-shape/`):
  `codeunit 80 RealById` and `codeunit 50100 "80"` (the named one declares `procedure P()` that id-80 lacks); a caller
  `var C: Codeunit "80"; C.P()` → the fresh edge must target the NAMED codeunit, not id 80 (`Evidence::Source`, exact id).
- [ ] **Step 2: Run — fail** (both collapse to id-80).
- [ ] **Step 3: Implement** — change `ParsedType::Object{kind, name: String}` → `ParsedType::Object{kind, object_ref:
  ObjectRef}`; `parse_object_kind_type` (`:883-888`) classifies quoted-vs-bare exactly as `classify_type_text`'s Record
  arm does (leading `"` ⇒ `ObjectRef::Name`; else attempt `i64` ⇒ `ObjectRef::Id`, else `Name`); replace
  `resolve_object_name_lc`'s `parse::<i64>()` branching with `index.resolve_object_ref(graph, from_object.id, kind,
  &object_ref)` (kind-generic, already exists) — set the resolved `id: Some` in the `Object` receiver up front and drop
  the redundant second lookup at `resolver.rs:1082`. Update the two `ParsedType::Object` test constructors (`:1202`, `:1213`).
- [ ] **Step 4: Run — pass.** (WITH `CDO_WS`) `cdo_l3_semantic_audit_no_fresh_wrong` — `genuine_wrong` stays 0, rate/fresh_missing
  unchanged (dormant, like I1; digit-named objects ~never in real BC).
- [ ] **Step 5: rustfmt + clippy + (no-CDO) test + commit** — `fix(resolve): shape-preserving object-typed declared-var
  resolution (ParsedType::Object → ObjectRef; Codeunit 80 vs "80")`.

---

### Task 3: Fresh-native `UnknownReason` diagnostic + stratified `aldump` breakdown

**Files:** Modify `src/program/resolve/edge.rs` (the `UnknownReason` enum + carry it), `src/program/resolve/resolver.rs`
+ `full.rs` (tag the 13 decline sites), `src/bin/aldump.rs` (stratified breakdown); Test (a fixture-corpus stratified
assertion + CDO breakdown print). **DIAGNOSTIC ONLY — the real-`unknown` COUNT is UNCHANGED; no reclassification.**

**Interfaces (Produces):** `pub enum UnknownReason { CompoundReceiver, UntrackedReceiver, UnclassifiedCallee,
OverloadAmbiguous, BuiltinPrecedenceCollision, WithScopeGuard, CodeunitTableNoExcluded, ReportRecExcluded,
ProtectedNotVisible, LocalNotVisible, InternalNotVisible, CatalogMiss, ReceiverOutOfClosure, MemberNotFound }`
(fresh-native, in `edge.rs`; `derive(Ord)` for stable ordering; extend/merge as the 13 sites dictate — NO catch-all
"BareUnresolved forgot-to-tag" bucket). **Carry it as a PAYLOAD on the evidence — `Evidence::Unknown(UnknownReason)`**
(gpt/gemini round-1), NOT `Option<UnknownReason>` on `Route`: the payload forces every construction to supply a reason
at compile time (exhaustive by construction) and adds no per-edge memory to the ~98% healthy edges. Rippling every
`Evidence::Unknown` match is the POINT (it surfaces every untagged site). Provide required-reason constructors
`member_unknown_route(reason, …)` / `unknown_route(reason, …)`; delete/privatize any zero-arg unknown constructor.

- [ ] **Step 1: Write the failing stratified test** — a small fixture corpus (or reuse existing `ws-*` fixtures) whose
  Unknown edges span ≥4 distinct reasons; assert `aldump --program-call-graph-stats` (or the new
  `--program-unknown-breakdown`) prints a per-`UnknownReason` count that sums to the total `unknown` count (the
  stratification is EXHAUSTIVE — every Unknown edge has exactly one reason; a `sum == unknown` invariant test).
- [ ] **Step 2: Run — fail** (no reason field / no breakdown).
- [ ] **Step 3: Implement** — change `Evidence::Unknown` → `Evidence::Unknown(UnknownReason)` in `edge.rs`; update every
  `Evidence::Unknown` construction/match (the compiler enumerates them — that is the exhaustiveness guarantee). At EACH of
  the 13 `member_unknown_route()`/`unknown_route()` call sites in `resolver.rs`/`full.rs`, supply the site-specific reason
  (grounding mapped them 1:1). `UnknownReason` gets a stable `as_str()` (camelCase matching aldump style) — render via that,
  NEVER `Debug`.
  **SERIALIZATION BOUNDARY (gpt/gemini round-2 — the critical new risk):** the anonymized semantic goldens
  (`cdo-anon.json` etc.) + the semantic-audit path serialize `Evidence`; a unit→tuple variant would flip serde `"Unknown"`
  → `{"Unknown":"…"}` and HARD-FAIL `cdo_l3_semantic_audit_no_fresh_wrong` on the historic goldens. So: add
  `Evidence::kind()` (projects `Unknown(_)` → the `"Unknown"` KIND) and make ALL semantic-golden/audit serialization +
  comparison use the KIND projection, NOT the payload — the reason lives ONLY in the `aldump` diagnostic breakdown. Confirm
  the committed semantic goldens are **byte-identical, NO regen** (the payload never touches them). Do NOT add a
  `LegacyUnspecified` catch-all reason.
  Surface the stratified breakdown via a `BTreeMap<UnknownReason, usize>` (deterministic) in `aldump --program-call-graph-stats`
  (`unknownByReason: {...}`) or a new `--program-unknown-breakdown <ws>` flag. The `classify_obligation`/`ObligationOutcome::Unknown`
  path is UNCHANGED — the reason is a diagnostic annotation, the edge stays `Unknown` (COUNT + classification IDENTICAL). Add
  invariants: (i) **code-level** — every `ObligationOutcome::Unknown` edge carries a reason (pin it); (ii) **sum** —
  `sum(unknownByReason.values()) == ObligationOutcome::Unknown count`; (iii) **histogram byte-identical** before/after; (iv)
  **goldens byte-identical** (no regen — the `kind()` projection holds).
- [ ] **Step 4: Run — pass.** (WITH `CDO_WS`) run the breakdown on CDO: record the per-reason distribution of the residual
  (this is the deliverable — the precise characterization). Confirm `sum == unknown` and that
  `cdo_full_program_coverage_and_self_reported_metric` rate + the full histogram are UNCHANGED vs the pre-Task-3 state
  (diagnostic-only). Deterministic across two runs.
- [ ] **Step 5: rustfmt + clippy + (no-CDO) test + commit** — `feat(resolve): fresh-native UnknownReason diagnostic +
  stratified aldump unknown breakdown (charter §8; count unchanged)`.

---

### Task 4: Re-measure, document the residual breakdown + roadmap, CHANGELOG + charter

**Files:** Modify `tests/program_resolve_harness.rs` (ratchets only if a soundness fix moved the metric), `CHANGELOG.md`,
charter memory; Test.

- [ ] **Step 1: Full re-measure** (WITH `CDO_WS` + `ENFORCE_CDO_WS=1`, SINGLE tests): record the FINAL rate + `unknown`
  count — it should equal the pre-plan 346 baseline EXCEPT for any Task-1/2 justified soundness corrections (false-`Source`
  → honest `Unknown` may RAISE it; that is correct); Task 3 is count-preserving. Record the per-`UnknownReason` breakdown +
  `genuine_wrong`. Capture the breakdown table for the CHANGELOG + the next-plan roadmap.
- [ ] **Step 2: Ratchets** — TIGHTEN if a Task-1/2 soundness correction dropped the rate/count. If Task 1/2 RAISED the
  count (a false-`Source`→honest-`Unknown` correction), RAISE the ceiling to the new value WITH the soundness justification
  (a dated comment naming the corrected false-`Source` class) — the ratchet must NOT block a soundness fix; a rise here is
  a coverage-optics loss that BUYS a soundness gain. Add a `sum(unknownByReason) == unknown` invariant + the "every Unknown
  has a reason" invariant to the gate (the stratification stays exhaustive). Never loosen a ceiling to hide a regression;
  only adjust for a justified soundness correction.
- [ ] **Step 3: Run** — (no CDO) `cargo test --workspace` green incl. the stratification invariant; (WITH `CDO_WS`) all
  gates green, deterministic, `checked_sites > 0`.
- [ ] **Step 4: CHANGELOG (honest)** — Task 1 caller-identity-aware visibility (same-app-local object-scope + cross-app
  Protected); Task 2 object-typed declared-var shape-preservation; Task 3 the fresh-native `UnknownReason` diagnostic +
  the MEASURED per-reason breakdown of the 346. State plainly: this plan HARDENED soundness + STRATIFIED the residual;
  it did NOT reduce the real-`unknown` count (that is the next, data-driven plan). List the breakdown so the next plan
  can prioritize (the biggest reason bucket = the next lever). No `engine::l3` import (grep-guarded).
- [ ] **Step 5: Charter memory** — append a concise entry (soundness completion done; the 346 now stratified by reason;
  the per-reason breakdown → the next plan's prioritized burndown + the honest-taxonomy reclassification decision).
  Update `MEMORY.md` pointer if warranted.
- [ ] **Step 6: Commit** — `docs(resolve): soundness completion + Unknown stratification — 346 residual characterized by reason`.

---

## Roadmap — beyond this plan (data-driven, informed by Task 3's breakdown)

**`ObsoleteState = Removed` latent false-`Source` (gemini round-2):** an obsolete-removed member cannot link in AL, so
resolving a call to it is a false `Source`. During Task 1, CHECK whether `RoutineNode` carries `ObsoleteState`; if it
does, excluding removed members (with a new `UnknownReason::ObsoleteRemoved`) is a cheap soundness win to fold in; if it
requires ingest-tier changes, leave it here as a tracked latent vector — do NOT expand Task 1's scope for it.

Decide, WITH the per-reason data + a fresh external review: (1) **honest-taxonomy reclassification** — add
`ObligationOutcome::Ambiguous`/`MemberNotFound` (charter §5 sub-states) and route the GENUINELY-honest reasons
(overload-ambiguous, member-genuinely-absent, out-of-closure-external→`Opaque`) out of real-`unknown` — proven per-route
genuine, NOT laundering, each gated. (2) The **burndown levers** ranked by their measured reason-count: Codeunit-`TableNo`
bare implicit-`Rec`, `with`-scope RESOLUTION, same-arity-type overload DISPATCH (arg-type capture + match), Report
implicit-`Rec` (dataitem block-scope), catalog completeness (RecordRef/FieldRef/Framework misses). North-star (charter §8):
drive workspace-originated real-`unknown` to its provably-dynamic residual, risk-weighted by centrality.
