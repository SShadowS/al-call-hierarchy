# Plan 1B.2 Phase 4 — Polymorphic/Multicast fan-out (applicability-gated) Implementation Plan

> Status: v2 — rewritten after gpt-5.5 + gemini-3.1-pro BOTH returned NO-GO on v1. Convergent fatal
> flaws: (1) v1 treated an existence WITNESS as proof of dispatch APPLICABILITY (the spec §6.3 says it
> is not); (2) the `unverified_extra` "by construction" check validated the OBJECT (`implementers_of`),
> not the ROUTINE — a wrong-overload route inside a correct implementer passed; (3) events were unsound
> (L3 event graph unqualified as oracle, under-qualified canonical key, `Manual` parsed from the wrong
> place, fixture fallback shipping unverified Multicast edges). v2 makes a **method/trigger-level
> applicability proof** the spine (built FIRST, gates every fresh-only route), and **splits events into a
> separate sub-plan (Phase 4b)** — they need oracle-qualification + manual-binding-property +
> canonical-key + reachability-semantics work that is its own project.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Resolve Interface/Enum dispatch as `Polymorphic` fan-out and gate the already-built implicit-trigger `Multicast`, plus the Object/Enum instance-builtin wedge — each fresh-only route validated by a **route-level applicability proof** (not a witness alone), dual-run-gated against L3 on CDO. Events are DEFERRED to Phase 4b.

**Architecture:** Extends `src/program/resolve/`. The edge model + indexes exist (`DispatchShape::Polymorphic/Multicast`, `SetCompleteness::Partial`, `ResolveIndex.implementers_of`, `resolve_implicit_trigger`). Phase 4 v2 builds the **applicability layer FIRST** (Task 0) — per-edge-kind predicates that prove a fresh route is the correct dispatch target, IR-anchored — then enables each resolution slice (instance-builtin → interface → implicit-trigger), gating each immediately. A fresh-only route that PASSES applicability is `fresh_ahead`/matched; one that FAILS is `unverified_extra` (a real false edge).

**Tech Stack:** Rust (edition 2024, toolchain 1.96.0), the `al-call-hierarchy` crate, `src/engine/l3` as the read-only ORACLE.

**Source of truth:** `docs/superpowers/specs/2026-06-29-plan1b2-fresh-resolver-design.md` (§3.1 Polymorphic/Multicast, §6.3 witness≠applicability, §6.5 gate). Read it first.

## Honest scope statement (per the reviews)

This is a **scoped sub-phase**, NOT "Phase 4 complete" per spec §7 (which says "all filters removed"). Phase 4 closes the dual-run-gatable fan-out: **Interface Polymorphic, implicit-trigger Multicast, instance-builtins**. Still deferred (the gate MUST report their excluded counts; CHANGELOG must NOT claim full Phase-4/whole-program completion): **events** (Phase 4b), `regression_page_rec`, `regression_compound_receiver`, `regression_codeunit_implicit_rec`, and same-arity-type overload disambiguation (which, where it affects interface dispatch, is handled by routing ambiguous cases to Unknown — NOT fresh-ahead).

## Key facts grounding this plan (from the Phase-4 L3 map)

- **The applicability invariant (the spine — spec §6.3):** a `Witness` proves the route's target EXISTS; it does NOT prove the site dispatches to it. A fresh-only route is `fresh_ahead`/matched ONLY with a route-level applicability proof; else `unverified_extra` (or `divergence` if L3 bound differently). The proof is edge-kind-specific:
  - **Instance-builtin:** the receiver's inferred type is exactly an instance of the object KIND, AND the builtin catalog entry belongs to THAT kind's instance set (the per-kind set IS the kind-uniform applicability — every Page instance has `RunModal`; instance methods are uniform across a kind), AND name+arity match. Catalog-hit-for-the-receiver-kind = applicable.
  - **Interface route:** the implementer object implements the EXACT app-qualified interface (`ObjectNode.implements` contains it), AND the target routine is the implementer's implementation of THE CALLED interface MEMBER — i.e. the UNAMBIGUOUS name+arity match in that implementer. If the implementer has MULTIPLE same-name-same-arity candidates (overload ambiguity, since type-disambiguation is deferred) → the route is AMBIGUOUS → emit `Unknown`/blocked, NOT a confident fresh-ahead route.
  - **ImplicitTrigger route:** the target is the EXACT trigger for the operation on the base table OR a `TableExtension` of THAT table (`Insert`→OnInsert, `Modify`→OnModify, `Delete`→OnDelete, `Rename`→OnRename, `Validate(F)`→the field F's OnValidate); `Insert(false)` literal → NO edge; `Insert(var)` → `Condition::RunTriggerGuarded`.
- **L3 fails the same fresh-ahead cases** (so they're genuine improvements, not L3-parity): `PageVar.RunModal()` → L3 `MemberNotFound`; enum methods → L3 `EnumStatic` Unknown; enum interface-implementers → L3 metadata-only (no edge). Fresh resolving these = fresh-ahead, IF applicability holds.
- **Object instance-builtin wedge:** `resolve_member` `Object{kind,name_lc}` arm (`resolver.rs:624-666`) has no catalog fallback after `resolve_in_object` None. `member_catalog.rs` has `PAGE_INSTANCE`/`REPORT_INSTANCE` (no `QueryInstance`/`CodeunitInstance` `FrameworkKind` — add if `member_builtins.json` has Query/Codeunit-var instance methods that appear on CDO).
- **Interface dispatch:** L3 `resolve_interface_dispatch` (`call_resolver.rs:401-458`) → `objects_implementing` (CODEUNIT-only) → `DispatchKind::Interface`. Fresh `implementers_of` (`index.rs:193`, ALL kinds incl. enums). Fresh enum-implementer routes are fresh-ahead (L3 has none).
- **ImplicitTrigger Multicast** (`resolve_implicit_trigger`, `resolver.rs:419-472`) already built (Multicast over `table_extensions_of`); NOT gated (harness skips `RecordOp`, `differential.rs:1433`; `DispatchKind::ImplicitTrigger` excluded from scopes). L3 emits it in `ResolvedCalls.edges`. Gate = scope expansion + the operation-specific applicability proof.
- **`unverified_extra` is a structural no-op today** (`differential.rs:1619`, always 0). Task 0 wires it to the applicability layer: a fresh-only route failing its route-level applicability proof → `unverified_extra`.

## Global Constraints

- Rust edition 2024; toolchain 1.96.0. `rustfmt <file>` per-file — **never** `cargo fmt`.
- Stage only files each task names — **never** `git add -A`.
- CI gates: `cargo clippy --release --all-features -- -D warnings` (NO `--tests`), `cargo fmt --check`, `cargo test --workspace`. All must pass.
- **`src/engine/l3` + goldens are read-only ORACLE — do not modify.** No L3 dispatch LOGIC imported (clean-room).
- **Gate FIRST, resolver SECOND** (the v1 staging error): Task 0 builds the applicability layer + classification + asserts `unverified_extra == 0` BEFORE any new resolver fan-out lands. Each resolution slice (Tasks 1-3) lands WITH its applicability proof + gates immediately.
- **Witness ≠ applicability** (spec §6.3): a fresh-only route is `fresh_ahead`/matched ONLY with a route-level applicability proof; failing → `unverified_extra`; L3-bound-differently → `divergence`. Never auto-allow a witnessed superset.
- **Ambiguity → Unknown, not fresh-ahead:** any overload-ambiguous (multiple same-name-same-arity candidates) resolution → `Unknown`/blocked route, NOT a confident route. Do not categorize an ambiguous resolution as fresh-ahead.
- Determinism; CDO env-gated (`CDO_WS` = `U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud`, return early when unset, run WITH set); CHANGELOG honest (scoped sub-phase, excluded counts, no full-completion claim).
- **rust-analyzer diagnostics on `resolve/*` are NOT authoritative** (~18× confirmed) — only `cargo build`/`test`/`clippy`.

## File / module structure

| File | Responsibility |
|------|----------------|
| `src/program/resolve/applicability.rs` (create) | Task 0: per-edge-kind route applicability predicates (IR/graph-anchored). |
| `src/program/resolve/differential.rs` (modify) | Task 0: wire applicability into the FreshOnly bucketing (`fresh_ahead`/`unverified_extra`/`divergence`); gate scope expansion (Tasks 2-3). |
| `src/program/resolve/resolver.rs` (modify) | Tasks 1-2: instance-builtin fallback, Interface fan-out (ambiguity→Unknown). |
| `src/program/resolve/{receiver.rs,member_catalog.rs}` (modify) | Task 1: `QueryInstance`/`CodeunitInstance` if needed. |
| `tests/program_resolve_harness.rs` (modify) | Tasks 0-4: gate assertions + applicability unit tests + fixtures. |

---

### Task 0: Applicability layer + classification + `unverified_extra` teeth (FIRST — the spine)

**Files:** Create `src/program/resolve/applicability.rs`; modify `differential.rs`, `tests/program_resolve_harness.rs`. Test (unit + the existing CDO gates must still pass).

**Interfaces:**
- Produces `applicability.rs`. CRITICAL (both reviewers, round 2): the predicates take the CALL-SITE context (the called member + arity / the record-op shape), NOT just the target — else a route to a unique-but-WRONG routine in a correct object passes (a route to a unique `Baz()` in an `IFoo` implementer would falsely pass for a call to `IFoo.Bar()`):
  - `pub fn interface_route_applicable(iface_lc: &str, called_member_lc: &str, called_arity: usize, target: &RoutineNodeId, graph: &ProgramGraph) -> bool` — the target's OBJECT implements `iface_lc` (exact, via `ObjectNode.implements` lowercased) AND `target.name_lc == called_member_lc` AND `target.params_count == called_arity` AND it is the UNAMBIGUOUS such match in that object (exactly one routine of that (object, called_member_lc, called_arity); if multiple → NOT applicable → caller emits Unresolved). gemini confirmed name+arity uniqueness is sufficient: AL has NO explicit method-level interface wiring; the contract is an implicit public-signature match. Same-name-same-arity type-disambiguation is deferred to 1B.3 → those punt to not-applicable.
  - `pub fn implicit_trigger_route_applicable(ctx: &RecordOpCtx, target: &RoutineNodeId, graph, index) -> bool` where `RecordOpCtx { kind: RecordOpKind /*Insert|Modify|Delete|Rename|Validate*/, table: ObjectNodeId, field: Option<FieldId> /*Some for Validate*/, run_trigger: RunTrigger /*True|False|Guarded*/ }` — `run_trigger == False` → ALWAYS false (no edge for `Insert(false)`); the target is the correct trigger for `ctx.kind` (`Insert`→`oninsert` …; `Validate`→the OnValidate of `ctx.field` SPECIFICALLY, not any OnValidate) AND target's object is `ctx.table` OR a `TableExtension` of it. (`op: &str` was too weak — a Validate route to the wrong field's OnValidate must be NOT applicable.)
  - `pub fn instance_builtin_route_applicable(kind: ObjectKind, method_lc: &str) -> bool` — the method is in THAT kind's instance-builtin catalog set (Page→PAGE_INSTANCE etc.) — kind-uniform applicability (gpt confirmed `RunModal`-class methods are platform-surface, no per-subtype opt-out). NOTE: object-metadata-sensitive instance methods (`SetRecord`/`SetTableView` tied to a specific source table/dataitem) are NOT kind-uniform-safe for argument validity — Task 1 keeps those OUT of fresh-ahead (resolve to Unknown, not a confident Catalog route) until the object-specific table constraint is modeled; `RunModal`/`Run`/`Close`/`SaveAsPdf`-class category methods ARE in-scope.
  - (Event applicability is Phase 4b — not here.)
- Modifies the harness FreshOnly bucketing: a fresh-only fan-out route is classified by calling the matching predicate. PASS → `fresh_ahead_<kind>` (named, justified-extra). FAIL → `unverified_extra` (a real false edge). A site L3 bound to a DIFFERENT target → `divergence`. Wire the `unverified_extra` accumulator (remove the always-0).

- [ ] **Step 1: Write failing unit tests** (no CDO needed) — `interface_route_applicable`: a target in an implementer of the iface whose name+arity == the CALLED member → true; a target whose object does NOT implement the iface → false; a target that is a unique routine but whose name ≠ the called member (a `Baz()` route for a call to `Bar()`) → **false** (the site-context test gpt flagged); an AMBIGUOUS target (two same-name-same-arity routines) → false. `implicit_trigger_route_applicable`: `RecordOpCtx{Insert, T, None, True}` + OnInsert on base T → true; same + OnInsert on a TableExtension of T → true; + OnInsert on an UNRELATED table → false; + a wrong trigger (OnModify for Insert) → false; `RecordOpCtx{Validate, T, field=No., True}` + the `No.` OnValidate → true but + the `Name` OnValidate → **false** (specific field); `RecordOpCtx{Insert, T, None, False}` (Insert(false)) + any trigger route → **false**. `instance_builtin_route_applicable`: (Page, "runmodal") → true; (Page, "notamethod") → false; (Codeunit, "runmodal") → false.
- [ ] **Step 2: Run to verify they fail.**
- [ ] **Step 3: Implement `applicability.rs`** + wire the three predicates into the harness FreshOnly bucketing (`fresh_ahead_*` on pass, `unverified_extra` on fail). Add the report fields `fresh_ahead_instance_builtin`, `fresh_ahead_interface`, `fresh_ahead_enum_static`, and the now-live `unverified_extra`. At this point NO new resolver fan-out exists, so on the current CDO gates `unverified_extra` stays 0 (no fan-out routes yet) and the existing assertions hold — confirm.
- [ ] **Step 4: Run the existing Phase-2/3 CDO gates WITH `CDO_WS`** — confirm `regression_unexplained==0`, `evidence_overclaim==0`, `unverified_extra==0` STILL hold (the applicability layer is inert until Tasks 1-3 emit fan-out routes).
- [ ] **Step 5: rustfmt + clippy + `cargo test --workspace` + commit** — `feat(resolve): route applicability layer + unverified_extra teeth (Phase 4 Task 0)`.

---

### Task 1: Object/Enum instance-builtin wedge (applicability-gated)

**Files:** Modify `resolver.rs` (+ `receiver.rs`/`member_catalog.rs` if adding kinds); Test in `resolver.rs` + the CDO member gate.

**Interfaces:** Consumes `member_builtin_id`, `FrameworkKind`, `instance_builtin_route_applicable` (Task 0).

- [ ] **Step 1: Write failing tests** — `MyPage: Page "X"` + `runmodal` → `Route{Builtin(PageInstance::runmodal), Catalog}` AND `instance_builtin_route_applicable(Page,"runmodal")==true`; `Enum "Color"` var + `asinteger` → `Route{Builtin(Enum::asinteger), Catalog}`; an Object receiver + a real declared proc still → Source (unbroken); an Object receiver + neither → Unknown.
- [ ] **Step 2: Run to verify they fail.**
- [ ] **Step 3: Implement** the `Object{kind}` catalog fallback (Page→PageInstance, Report→ReportInstance, + Query/Codeunit instance kinds if `member_builtins.json` warrants) + the `EnumType{}` ENUM_VALUE arm. These are kind-uniform builtins (applicability holds by kind+catalog membership). EXCLUDE the object-metadata-sensitive methods (`SetRecord`/`SetTableView`-class, whose argument validity depends on the specific object's source table) from the catalog fast-path → leave them Unknown for now (gpt round-2 caveat); only the kind-uniform category methods (`RunModal`/`Run`/`Close`/`SaveAsPdf`/…) get the Catalog route.
- [ ] **Step 4: Run the member CDO gate WITH `CDO_WS`** — the new sites are `fresh_ahead_instance_builtin`/`fresh_ahead_enum_static` (L3 was MemberNotFound/EnumStatic; applicability passes by kind+catalog); `regression_unexplained==0`, `evidence_overclaim==0`, `unverified_extra==0`; `regression_enum_static` drains. Print the breakdown.
- [ ] **Step 5: Full gate + commit** — `feat(resolve): Object/Enum instance-builtin wedge, applicability-gated (Phase 4 Task 1)`.

---

### Task 2: Interface Polymorphic fan-out (ambiguity→Unknown, applicability-gated)

**Files:** Modify `resolver.rs`, `differential.rs`, `tests/program_resolve_harness.rs`, `CHANGELOG.md`; Test (unit + CDO).

**Interfaces:** Consumes `ResolveIndex.implementers_of`, `resolve_in_object`, `DispatchShape::Polymorphic`, `interface_route_applicable` (Task 0).

- [ ] **Step 1: Write failing tests** — an `i: Interface "IFoo"` + `Bar` where two codeunits implement IFoo each with a unique `Bar(arity)` → `(Polymorphic, Partial{ReverseDependentImplementers}, [2 Routine routes])`, each route `interface_route_applicable==true`; ONE implementer → 1 route; ZERO implementers → `(Polymorphic, Partial, [])` → HonestEmpty; **a known implementer A (resolves `Bar`) PLUS a known implementer B with an AMBIGUOUS `Bar` (two same-arity overloads) → `[Routine(A), Unresolved(B)]` — B emits a `Route{RouteTarget::Unresolved, Evidence::Unknown}` and is NOT dropped** (gemini round-2: silently dropping B is a reachability black hole — the site would look fully Resolved while B, which runs at runtime, vanishes); an enum implementer with a `Bar` → a route (fresh-ahead vs L3 codeunit-only).
- [ ] **Step 2: Run to verify they fail.**
- [ ] **Step 3: Implement** the `Interface{name_lc}` arm: `implementers_of(name_lc)` → for each implementer, resolve the called method via `resolve_in_object`. If it resolves UNAMBIGUOUSLY → a Routine route. **If a KNOWN implementer fails internal resolution (ambiguous same-arity overloads, or method absent) → emit `Route{RouteTarget::Unresolved, Evidence::Unknown}` for THAT implementer (a hard per-implementer failure, visible in the set) — do NOT drop it** (dropping falsely marks the site Resolved + hides runtime-reachable code). Collect → `(Polymorphic, Partial{ReverseDependentImplementers}, routes)`. Add `DispatchKind::Interface` to `project_l3_member_in_scope` (`differential.rs:1754`). Each fresh Routine route is checked by `interface_route_applicable(iface, called_member, called_arity, target, graph)` in the harness → `fresh_ahead_interface` (codeunit-impl matches L3; enum-impl is fresh-ahead) or `unverified_extra` (FAIL = a real bug); Unresolved routes are not applicability-checked (they claim nothing).
- [ ] **Step 4: Run CDO gate WITH `CDO_WS`** — `regression_unexplained==0`, `evidence_overclaim==0`, `unverified_extra==0`, `regression_interface` drains. INVESTIGATE any `unverified_extra > 0` (a fan-out emitting a non-applicable route — a real resolver bug to fix, NOT to relax). Print the breakdown.
- [ ] **Step 5: Full gate + commit** — `feat(resolve): Interface Polymorphic fan-out, ambiguity→Unknown, applicability-gated (Phase 4 Task 2)`.

---

### Task 3: ImplicitTrigger Multicast gating (operation-specific applicability)

**Files:** Modify `differential.rs`, `tests/program_resolve_harness.rs`; Test (env-gated CDO).

**Interfaces:** `resolve_implicit_trigger` (built) + `implicit_trigger_route_applicable` (Task 0) + the L3 oracle scope.

- [ ] **Step 1: Write the failing CDO gate test** — `phase4_implicit_trigger`: fresh `Multicast` ImplicitTrigger edges (from RecordOp sites) vs L3 `DispatchKind::ImplicitTrigger`. Assert `regression_unexplained==0`, `evidence_overclaim==0`, and every fresh trigger route passes `implicit_trigger_route_applicable` (→ matched/fresh_ahead, none `unverified_extra`); deterministic.
- [ ] **Step 2: Run to verify it fails.**
- [ ] **Step 3: Implement** — add `DispatchKind::ImplicitTrigger` to the L3 oracle scope; scope-gate the `RecordOp continue` skip (`differential.rs:1433`) so fresh RecordOp→ImplicitTrigger edges enter THIS gate pass; match on (caller-span, target-trigger-routine). Each fresh trigger route validated by `implicit_trigger_route_applicable` (op→trigger-name, target on base table or its extension). INVESTIGATE divergences (the Phase-2 per-field Validate over-count: a Validate route to ALL OnValidate triggers vs L3's specific field — categorize as a known `fresh_ahead_validate_fanout` or fix by capturing the field arg; do NOT let it become `unverified_extra` without inspection — but if a route targets a trigger on an UNRELATED table, that IS `unverified_extra`).
- [ ] **Step 4: Run WITH `CDO_WS`** — asserts pass.
- [ ] **Step 5: Full gate + commit** — `feat(resolve): ImplicitTrigger Multicast gating, operation-specific applicability (Phase 4 Task 3)`.

---

### Task 4: Whole-(in-scope)-corpus Phase-4 gate + honest framing

**Files:** Modify `tests/program_resolve_harness.rs`, `CHANGELOG.md`, the charter memory note; Test (env-gated CDO).

**Interfaces:** Consolidates Tasks 1-3 into the Phase-4 gate report.

- [ ] **Step 1: Write the failing consolidated CDO gate** — `phase4_fanout_matches_or_beats_l3`: over the Phase-4 in-scope subset (Member + Interface + ImplicitTrigger), assert `regression_unexplained==0`, `evidence_overclaim==0`, `unverified_extra==0` (the applicability layer's teeth — any non-applicable fresh route is a hard failure), divergence ≤ its adjudicated cap, deterministic. Print the FULL categorized breakdown incl. the `fresh_ahead_*` buckets AND the still-EXCLUDED counts (events, page_rec, compound_receiver, codeunit_implicit_rec) so the scope is explicit.
- [ ] **Step 2: Run to verify it fails / passes** (it should largely pass after Tasks 1-3; this task consolidates + adds the excluded-count reporting + the honest framing).
- [ ] **Step 3: Implement** the consolidated report + the excluded-count printout. CHANGELOG: state Phase 4 closes Interface Polymorphic + ImplicitTrigger Multicast + instance-builtins, dual-run-gated with route-level applicability proofs; events + the 3 receiver-gap buckets remain (Phase 4b / 1B.3); do NOT claim full whole-program completion.
- [ ] **Step 4: Run WITH `CDO_WS`** — confirm the consolidated gate green + the excluded counts reported.
- [ ] **Step 5: Full gate + commit** — `feat(resolve): consolidated Phase-4 fan-out gate + honest scope framing (Phase 4 Task 4)`.

---

## Roadmap — Phase 4b (events) + 1B.3

**Phase 4b — Event flow** (its own plan, because the reviews showed it needs real design work):
- QUALIFY L3's `event_graph` as an oracle FIRST (what it models: manual binding, cross-app subscribers, conditions, integration vs business events; what it MISSES) — or mark EventFlow non-shipping until qualified.
- Populate `subscribers_of` from `RoutineDecl.attributes_parsed` (EventSubscriber attr: publisher object-type + name + event + element), resolving the publisher SIGNATURE-aware (not just by name — overload-blind binding is a flaw).
- Derive `Condition::ManualBinding` from the subscriber codeunit's `EventSubscriberInstance = Manual` PROPERTY (a codeunit property, NOT an attribute arg); `SkipOnMissingLicense/Permission` from the attribute booleans. Support MULTIPLE conditions per route (`Vec<Condition>` / bitset — `Option<Condition>` is too narrow).
- Canonical event key = (publisher event-routine NodeId, publisher object identity, event name, element, subscriber routine NodeId, originating-attribute id, condition set) — NOT just (publisher, subscriber).
- Reachability honesty: a `Manual` subscriber is a CONDITIONAL may-edge (dead unless `BindSubscription`), NOT an unconditional Multicast AND-route. Either a distinct handling or a downstream contract that Multicast iterators exclude `ManualBinding` by default.
- Event emission must NOT land as normal shipping graph output without the qualified oracle/gate (no fixture-only shipping of the most fan-out-heavy edge kind).

**1B.3** — full SymbolReference ABI cross-check (verify `Abi`/`Opaque` routes); deep re-baseline; retire L3 oracle. Carry-over: `regression_page_rec` (Page implicit-Rec source-table → ObjectNode), `regression_compound_receiver` (chained receiver propagation), `regression_codeunit_implicit_rec`, same-arity-type overload disambiguation (`disambiguate_by_arg_types`-equivalent), the 17 Cat-D different-named-target divergences.

## Self-Review

- **Review fixes incorporated:** (1) applicability proof (method/trigger/kind-level, IR-anchored) gates every fresh-only route — Task 0 FIRST, used by Tasks 1-3; a witness alone NEVER suffices. (2) `unverified_extra` teeth are method/trigger-level (a wrong-overload route inside a correct implementer FAILS `interface_route_applicable` → `unverified_extra`), not object-membership. (3) ambiguous overload resolution → Unknown, never fresh-ahead. (4) events SPLIT to Phase 4b with the oracle-qualification + manual-property + canonical-key + reachability work spelled out. (5) staging is gate-first (Task 0), each slice gated immediately. (6) honest scoped-subphase framing + excluded-count reporting (not "Phase 4 complete").
- **Spec coverage:** §3.1 Polymorphic (Interface) → Task 2; Multicast (ImplicitTrigger) → Task 3; §6.3 witness≠applicability → Task 0 (the spine); §6.5 gate → Task 4; the opus instance-builtin wedge → Task 1. EventFlow (§5.3) → Phase 4b (honest deferral, not silent).
- **Placeholder scan:** the "read member_builtins.json / mirror L3 logic" steps name the exact file. No `TODO`.
- **Type consistency:** `interface_route_applicable`/`implicit_trigger_route_applicable`/`instance_builtin_route_applicable` (Task 0) → Tasks 1-3; `DispatchShape::Polymorphic`/`SetCompleteness::Partial` (existing) → Tasks 2-3; the `fresh_ahead_*` + live `unverified_extra` report fields (Task 0) → Tasks 1-4.
