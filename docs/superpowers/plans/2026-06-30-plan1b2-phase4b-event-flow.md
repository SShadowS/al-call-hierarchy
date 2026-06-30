# Plan 1B.2 Phase 4b — Event flow (publisher→subscriber EventFlow edges) Implementation Plan

> Status: v2 — rewritten after gpt-5.5 GO-WITH-CHANGES + gemini-3.1-pro NO-GO. Convergent fixes folded in:
> (1) a **Manual subscriber must NOT score plain `Resolved`/unconditional-Multicast** — it needs a distinct
> conditional obligation + an opt-in reachability contract (else it's a reachability lie); (2) the
> `l3_only==0` gate is **mathematically impossible** as written because fresh is arity-aware while L3 is
> name-only (L3's wrong-arity false-positives would land in `l3_only`) → gate on `l3_only` MINUS a
> machine-derived `l3_false_positive_arity_mismatch` bucket; (3) publisher overload resolution must NOT
> "first-match" — ambiguous → `unresolved_ambiguous`, never guess; (4) Task-5 teeth must be INDEPENDENT of
> the index's own parse (re-derive from raw IR / structural param-map), not circular; (5) `RoutineNode`
> carries `Vec<ParsedSubscriberArgs>` (multiple `[EventSubscriber]` per handler), not `Option`; (6) the gate
> must FAIL on unprojectable events + machine-categorize `fresh_only` (asserted-zero uncategorized) + use a
> STRUCTURAL key (publisher id + event + arity + subscriber id), not only L3's opaque `stable_event_id` hash.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Model AL event publisher→subscriber `EventFlow` Multicast edges — `subscribers_of` from `[EventSubscriber]` attributes (signature-aware, ambiguity→unresolved), per-route conditions (`ManualBinding` from the codeunit property, `SkipOnMissing*` from attr args), Manual subscribers represented as **conditional may-edges that default reachability does NOT traverse**, and a **structural** dual-run event gate over the subset L3 can oracle (L3's arity-blind false-positives reconciled), the rest fixture-validated. NO fixture-only shipping of the gatable core.

**Architecture:** Extends `src/program/resolve/`. Events are PUBLISHER-anchored (one edge per publisher event routine, routes = subscribers) — a NEW sweep. L3's event graph (`src/engine/l3/event_graph.rs`, read-only ORACLE) gates the CORE pairing via a STRUCTURAL key; L3 GAPS (conditions, element filter, multiple-attrs, InternalEvent, arity-blindness) are reconciled/fixture-validated/non-shipping — honestly labelled + machine-categorized. The `AttributeIr.args` ExprId↔AlFile coupling is resolved by parsing subscriber args at `extract_nodes` (which has `AlFile`) into flat `Vec<ParsedSubscriberArgs>` on `RoutineNode`.

**Tech Stack:** Rust (edition 2024, toolchain 1.96.0); `src/engine/l3/event_graph.rs` (`build_event_graph`, `project_event_graph`, `L3EventGraphProjection`) as ORACLE.

**Source of truth:** `docs/superpowers/specs/2026-06-29-plan1b2-fresh-resolver-design.md` (§3.1 Multicast, §5.3 event flow + conditionality, §6.3 witness≠applicability, §6.5 gate). Phase-4b was split from Phase 4 on the reviewers' insistence; v2 folds their round-1 findings.

## Honest gating split (the spine — from L3 event-graph oracle qualification)

| Aspect | Validation | Why |
|--------|-----------|-----|
| Publisher→subscriber pairing (`resolution=="resolved"`) | **DUAL-RUN-GATED** (structural key) | L3 indexes these reliably |
| L3's arity-blind FALSE POSITIVES (L3 links a wrong-arity subscriber) | **RECONCILED** — two-stage join: arity-agnostic primary key (`pair_l3_only==0`), then arity compared WITHIN matched groups (`l3_false_positive_arity_mismatch`) | fresh correctly rejects; L3 over-links (name-only). Arity is NOT in the join key (else L3-arity-unknown double-misses) |
| `ManualBinding` condition | **FIXTURE** | L3 has ZERO signal (`EventSubscriberInstance` unread) |
| `SkipOnMissingLicense`/`Permission` | **FIXTURE** | L3 doesn't read attr args 4/5 |
| Element/field filter (arg 3) | **FIXTURE** (parsed + carried, not yet route-matched) | L3 drops it from `EventEdge` |
| Multiple `[EventSubscriber]` per handler | **FIXTURE** (fresh-wins) | L3 reads only the FIRST → under-counts |
| `InternalEvent` event-kind field | **NON-SHIPPING** comparison | L3 classes it `"unknown"` |

The dual-run gate asserts `l3_only - l3_false_positive_arity_mismatch == 0` (no genuinely-correct L3 subscriber that fresh misses). `fresh_only` is MACHINE-CATEGORIZED (every entry gets exactly one derived reason: `l3_maybe_upgrade` / `multiple_attr_l3_gap` / `arity_precise_fresh_win` / `internal_event_non_shipping`); `fresh_only_uncategorized` is ASSERTED 0 (a hallucinated subscription cannot hide in a free-form bucket).

## Key facts grounding this plan

- **L3 oracle** (`event_graph.rs`): publishers via `[IntegrationEvent]`/`[BusinessEvent]` (InternalEvent→"unknown"); `parse_subscriber_attribute` reads args 0-3 not 4/5; publisher resolution NAME-ONLY (`object_by_type_name`+`encode_event_id`, no arity → **L3 over-links overloads**). `PEventEdge { event_id (stable {objId}::{event}::{sigHash}), subscriber_routine_id (stable), subscriber_app_id, resolution }`; `PEventSymbol` carries publisher object id + event name (joinable). `project_event_graph`→`L3EventGraphProjection`. Cross-app supported.
- **Attribute IR** (`al-syntax/src/ir/decl.rs`): `AttributeIr { name, raw, args: Vec<ExprId> }`; `RoutineDecl.attributes_parsed: Vec<AttributeIr>` (a handler can have MULTIPLE `[EventSubscriber]`). Args are `ExprId` into `AlFile.ir.exprs` — need `&AlFile`. `EventSubscriberInstance=Manual` is in `ObjectDecl.properties`, NOT an attr arg.
- **Edge model** (`edge.rs`): `EdgeKind::EventFlow`, `DispatchShape::Multicast`, `OpenWorldReason::ReverseDependentSubscribers`, `Condition::{ManualBinding, SkipOnMissingLicense, SkipOnMissingPermission}` EXIST. `Route.condition: Option<Condition>` → widen to `conditions: Vec<Condition>`. `classify_obligation` exists.
- **THE COUPLING RISK:** `ResolveIndex::build(&ProgramGraph)` has no `AlFile` → parse subscriber args at `extract_nodes` into flat `Vec<ParsedSubscriberArgs>` on `RoutineNode`.
- **Manual binding semantics:** a Manual subscriber does NOT fire on publish unless `BindSubscription` is called at runtime → it is a CONDITIONAL may-edge. Modeling it as an unconditional Multicast route that scores plain `Resolved` is a REACHABILITY LIE (a graph walker keyed on obligation traverses it blindly). `bindsubscription`/`unbindsubscription` are catalog builtins; runtime bind-state is out of scope (1B.3).

## Global Constraints

- Rust edition 2024; toolchain 1.96.0. `rustfmt <file>` per-file — never `cargo fmt`. Stage only named files — never `git add -A`.
- CI gates: `cargo clippy --release --all-features -- -D warnings` (NO `--tests`), `cargo fmt --check`, `cargo test --workspace`. All pass.
- **`src/engine/l3` + goldens read-only ORACLE.** No L3 event LOGIC imported. The structural key is fresh-owned; L3's `stable_event_id` is consumed only via its projection for a SECONDARY cross-check (never reproduced as the sole join — see Task 4).
- **NO fixture-only shipping of the gatable core.** The pairing lands behind the dual-run gate (`l3_only - arity_fp == 0`).
- **Manual ≠ unconditional reachability:** a `ManualBinding` route is emitted but its obligation/reachability is DISTINCT — default reachability traversal MUST NOT traverse it. ENFORCEMENT (round-2 reviewer-mandated, beyond docs+one test): the reachability accessors are the ONLY sanctioned path, every EXISTING `edge.routes` consumer is audited+classified (resolution-context = may read all; reachability-context = MUST use `default_reachable_routes`), any reachability consumer is migrated, and a regression test exercises the REAL reachability entrypoint (not just the helper). Prefer compiler-level friction: if the churn is bounded, make `Edge.routes` non-`pub` with named accessors (`all_routes()` for resolution/gate, `default_reachable_routes()`/`may_reachable_routes()` for reachability); else keep it `pub` but add the audit + the named reachability accessors + a module-doc contract.
- **Gate must FAIL on unprojectable events** (no silent drop) + machine-categorize `fresh_only` (`fresh_only_uncategorized == 0`).
- **Publisher overload ambiguity → `unresolved_ambiguous`**, never a guessed edge (a wrong publisher poisons reachability from the wrong source).
- Determinism; CDO env-gated (`CDO_WS`); CHANGELOG honest. rust-analyzer diagnostics on resolve/* NOT authoritative — only cargo.

## File / module structure

| File | Responsibility |
|------|----------------|
| `src/program/resolve/edge.rs` (modify) | Task 0: `conditions: Vec<Condition>` + `Route::fires_by_default()` + the conditional obligation distinction. |
| `src/program/resolve/event.rs` (create) | Task 1: `parse_event_subscriber_ir`, `is_event_publisher`, `read_event_subscriber_instance`, `ParsedSubscriberArgs`. |
| `src/program/{node.rs,node_extract.rs}` (modify) | Task 2: `Vec<ParsedSubscriberArgs>` on `RoutineNode`. |
| `src/program/resolve/index.rs` (modify) | Task 2: `subscribers_map` + `subscribers_of` (ambiguity→unresolved). |
| `src/program/resolve/resolver.rs` (modify) | Task 3: publisher-anchored `emit_event_flow_edges`. |
| `src/program/resolve/differential.rs` (modify) | Task 4-5: structural event matcher + the reconciled gate + independent teeth. |
| `tests/program_resolve_harness.rs` + `tests/fixtures/events/` | Tasks 1-5. |

---

### Task 0: Conditions `Vec` + Manual conditional-obligation + reachability contract + oracle note

**Files:** Create `docs/superpowers/notes/2026-06-30-l3-event-oracle-qualification.md`; modify `src/program/resolve/edge.rs` + every `Route` constructor (`resolver.rs`); Test.

**Interfaces:** Produces `conditions: Vec<Condition>`, `Route::fires_by_default() -> bool` (false iff `conditions` contains `ManualBinding`), and the obligation distinction so a Manual-only edge is NOT plain `Resolved` for reachability.

- [ ] **Step 1:** Write the oracle-qualification note — from `event_graph.rs` (cite lines): what L3 models vs the gaps (Manual/Skip*/element/multiple-attrs/InternalEvent/arity-blindness), and the per-aspect validation decision (the table above). Governs Tasks 4-5.
- [ ] **Step 2: Write failing tests** — (a) a `Route` holds `vec![ManualBinding, SkipOnMissingLicense]`; (b) `Route::fires_by_default()` is false when `conditions` contains `ManualBinding`, true otherwise (incl. SkipOnMissing* — those fire-by-default, may runtime-skip); (c) an Edge whose ONLY real routes are Manual classifies to a DISTINCT obligation (`ConditionalResolved` or equivalent — NOT plain `Resolved`), while an Edge with ≥1 fires-by-default real route is `Resolved`; (d) the reachability contract: a `default_reachable_routes(edge)` helper returns only fires-by-default routes, an opt-in `may_reachable_routes(edge)` returns all — assert default EXCLUDES the Manual route, may INCLUDES it.
- [ ] **Step 3: Run — fail.**
- [ ] **Step 4: Implement** — widen `Route.condition` → `conditions: Vec<Condition>` (all live callers `None`→`vec![]`). Add `Route::fires_by_default()`. Add the obligation distinction in `classify_obligation`: an edge whose real routes are ALL non-fires-by-default → a new `ObligationOutcome::ConditionalResolved` (add the variant; update `real_unknown_rate`/any matcher to treat it as resolved-for-resolution but flag-able for reachability — document). Add `default_reachable_routes`/`may_reachable_routes` helpers (+ `all_routes()` for resolution/gate). **AUDIT every existing `edge.routes` reader** (grep): classify each as resolution-context (gate/matcher/classify_obligation — may read all) or reachability-context (must migrate to `default_reachable_routes`); migrate any reachability reader. Prefer making `Edge.routes` non-`pub` with the named accessors if the churn is bounded; else keep `pub` + the accessors + a module-doc contract. Add a regression test through a representative reachability path proving Manual is excluded by default, included opt-in.
- [ ] **Step 5: Run** + `cargo test --workspace` + clippy + commit — `refactor(resolve): Route.conditions Vec + Manual conditional-obligation + reachability contract (Phase 4b Task 0)`.

---

### Task 1: Event attribute parsing (`event.rs`)

**Files:** Create `src/program/resolve/event.rs` (+ `pub mod event;`); Test (unit).

**Interfaces:** `parse_event_subscriber_ir(attr: &AttributeIr, ir: &Ir) -> Option<ParsedSubscriberArgs>` (`ParsedSubscriberArgs { publisher_object_type, publisher_name, event_name, element: Option<String>, skip_on_missing_license: bool, skip_on_missing_permission: bool }`); `is_event_publisher(decl: &RoutineDecl) -> Option<PublisherKind>` (Integration/Business/Internal); `read_event_subscriber_instance(obj: &ObjectDecl) -> bool`.

- [ ] **Step 1: Write failing unit tests** — `[EventSubscriber(ObjectType::Codeunit, Codeunit::"Pub", 'OnAfterX', '', true, false)]` → the parsed args (license=true, permission=false, element=None); a TWO-`[EventSubscriber]` handler → `parse` on each AttributeIr yields both; `is_event_publisher([IntegrationEvent])` → Some(Integration); `read_event_subscriber_instance` (Manual) → true.
- [ ] **Step 2: Run — fail.**
- [ ] **Step 3: Implement** — match `ir.expr(id)` ExprKinds (QualifiedEnum/Member/DatabaseReference/Literal(Text/Bool); absent → defaults). Clean-room.
- [ ] **Step 4: Run — pass.**
- [ ] **Step 5: rustfmt + clippy + commit** — `feat(resolve): event attribute + publisher-kind + EventSubscriberInstance parsing (Phase 4b Task 1)`.

---

### Task 2: `subscribers_of` population (`Vec` args, ambiguity→unresolved)

**Files:** Modify `src/program/node.rs` + `node_extract.rs` (`Vec<ParsedSubscriberArgs>` on `RoutineNode`), `src/program/resolve/index.rs`; Test (unit, synthetic graph).

**Interfaces:** `ResolveIndex.subscribers_of(publisher: &RoutineNodeId) -> &[SubscriberEntry]` where `SubscriberEntry { subscriber: RoutineNodeId, conditions: Vec<Condition>, element: Option<String> }`. Also `ResolveIndex.ambiguous_subscriptions() -> &[AmbiguousSub]` (subscriptions whose publisher overload couldn't be disambiguated — for the gate, NOT emitted as edges).

- [ ] **Step 1: Write failing tests** — publisher CU "Pub" `[IntegrationEvent] OnAfterX()`; subscriber CU "Sub" (`EventSubscriberInstance=Manual`) handler `[EventSubscriber(Codeunit, Codeunit::"Pub", 'OnAfterX', '')]` → `subscribers_of(pub_onafterx)` = `[{ subscriber, conditions:[ManualBinding], element:None }]`; a handler with TWO `[EventSubscriber]` to two different events → TWO entries (multiple-attr); `SkipOnMissingLicense=true` → conditions include it; **a publisher with TWO `OnAfterX` overloads both satisfying the arity bound → NO emitted entry, recorded in `ambiguous_subscriptions` (Rule: never first-match-guess)**; unresolvable publisher → no entry, no panic.
- [ ] **Step 2: Run — fail.**
- [ ] **Step 3: Implement** — (a) at `extract_nodes` (has `&AlFile`): for each `[EventSubscriber]` AttributeIr (ALL of them) → `parse_event_subscriber_ir`; store `Vec<ParsedSubscriberArgs>` + the manual-instance bool on `RoutineNode`. (b) `ResolveIndex::build` (no AlFile): for each subscriber arg, `graph.resolve_object` (closure-scoped) → `routines_in_object(event_name)` filtered to publisher-attr routines → candidates with `params_count >= subscriber arity`. ZERO → drop; EXACTLY ONE → `SubscriberEntry`; **MORE THAN ONE → `ambiguous_subscriptions` (NOT an edge; if a strictly-better arity/type match exists, prefer it, else ambiguous)**. conditions = [ManualBinding if manual-instance] + [Skip* from flags]. Deterministic (sort by subscriber RoutineNodeId).
- [ ] **Step 4: Run — pass.**
- [ ] **Step 5: rustfmt + clippy + `cargo test --workspace` + commit** — `feat(resolve): subscribers_of (Vec args, signature-aware, ambiguity->unresolved) (Phase 4b Task 2)`.

---

### Task 3: EventFlow edge emission (publisher-anchored)

**Files:** Modify `src/program/resolve/resolver.rs` (`emit_event_flow_edges`); Test (unit).

**Interfaces:** Consumes `subscribers_of`. Produces `EventFlow` `Edge`s.

- [ ] **Step 1: Write failing tests** — for the Task-2 graph: ONE `Edge{ from: pub_onafterx, kind: EventFlow, shape: Multicast, completeness: Partial{ReverseDependentSubscribers}, routes: [Route{ Routine(sub), Source, conditions:[ManualBinding] }] }`; the route is NOT in `default_reachable_routes(edge)` (Manual) but IS in `may_reachable_routes(edge)`; a publisher with ZERO subscribers → empty routes → HonestEmpty; witness = subscriber SourceSpan.
- [ ] **Step 2: Run — fail.**
- [ ] **Step 3: Implement** `emit_event_flow_edges(graph, index, body_map) -> Vec<Edge>` — for each publisher event routine: SiteId from the publisher routine name-origin span; routes = `subscribers_of` → `Route{ Routine(sub), tier_evidence (or AbiSymbol for SymbolOnly), conditions, SourceSpan }`; `(EventFlow, Multicast, Partial{ReverseDependentSubscribers})`. Wire into the edge-assembly path. Deterministic.
- [ ] **Step 4: Run — pass.**
- [ ] **Step 5: rustfmt + clippy + `cargo test --workspace` + commit** — `feat(resolve): publisher-anchored EventFlow Multicast emission (Phase 4b Task 3)`.

---

### Task 4: Structural dual-run event gate (arity-FP reconciled, unprojectable-fail)

**Files:** Modify `src/program/resolve/differential.rs`, `tests/program_resolve_harness.rs`, create `tests/fixtures/events/`; Test (fixtures + env-gated CDO).

**Interfaces:** Consumes the EventFlow edges + L3's `project_event_graph`.

- [ ] **Step 1: Write the failing tests** — (fixture) a 2-app workspace: a Manual subscriber, a SkipOnMissingLicense subscriber, a multiple-`[EventSubscriber]` handler, AND **an overloaded publisher where L3 (arity-blind) links a wrong-arity subscriber** → assert fresh's structural projection + the `l3_false_positive_arity_mismatch` categorization. (env-gated CDO) `phase4b_event_flow`: assert `pair_l3_only == 0` (arity-agnostic recall — stage 1), `fresh_only_uncategorized == 0`, `fresh_unprojectable == 0`, `l3_unprojectable == 0`, `l3_regression == 0` (stage-2 arity disagreement on a matched/selected edge), deterministic; PRINT the full machine-categorized breakdown (+ `l3_false_positive_arity_mismatch`, `l3_arity_unknown` counts) + projection coverage counters.
- [ ] **Step 2: Run — fail.**
- [ ] **Step 3: Implement** — a **TWO-STAGE join** (round-2 fix: arity is NOT in the primary key — else an L3 arity-unknown vs fresh arity-N mismatch double-misses into `l3_only` + `fresh_only_uncategorized` and fails the gate spuriously):
  - PRIMARY key = arity-AGNOSTIC `PairKey = (publisher_stable_obj_id, event_name_lc, subscriber_stable_routine_id)` — fresh-OWNED. `project_fresh_events(&[Edge]) -> Vec<FreshEventRow>` (`FreshEventRow { pair: PairKey, publisher_arity, l3_xref_hash: Option<String> }`); a route that can't produce a full `PairKey` → `fresh_unprojectable` (HARD FAIL).
  - `project_l3_events(workspace) -> Vec<L3EventRow>` — from `project_event_graph()` filtered `resolution=="resolved"`, JOIN `PEventEdge`↔`PEventSymbol` (publisher obj id + event name); `publisher_arity: Option<usize>` (None when L3's symbol doesn't expose it). An L3 row that can't produce a `PairKey` → `l3_unprojectable` (HARD FAIL).
  - STAGE 1 — group both sides by `PairKey`, set-diff on the KEYS. A `PairKey` present in L3 but absent in fresh → a `pair_l3_only`; absent in L3, present in fresh → `pair_fresh_only`.
  - STAGE 2 — within `PairKey`s present on BOTH sides, compare arities: if L3 has MULTIPLE rows for the key (overloads) but fresh has ONE, the unselected L3 arities → `l3_false_positive_arity_mismatch` (L3 over-linked, fresh correctly picked one); if L3's `publisher_arity` is `None` (`l3_arity_unknown`) → ACCEPT the pair match as success, no penalty (L3 simply can't disambiguate). A genuine arity disagreement where BOTH expose arity and they differ on the SELECTED edge → `l3_regression` (investigate).
  - Gate: `pair_l3_only == 0` (every L3-resolved (pub,event,sub) is matched by fresh — the real recall guard; an L3 row that fresh entirely lacks is a regression, NOT an arity-FP, since arity-FP only applies WITHIN a matched key). `l3_arity_unknown` rows that DO match a fresh pair are accepted (not penalized, not a coverage hole — they ARE projected + matched). Machine-categorize every `pair_fresh_only` (`l3_maybe_upgrade` / `multiple_attr_l3_gap` / `internal_event_non_shipping`); `fresh_only_uncategorized == 0` asserted. `l3_xref_hash` mismatch on a matched pair → REPORTED, never silently dropped. INVESTIGATE any `pair_l3_only > 0` / `l3_regression > 0` (FIX, do NOT relax).
- [ ] **Step 4: Run WITH CDO_WS** — the asserts pass; print the breakdown + coverage counters; confirm the body ran.
- [ ] **Step 5: Full gate + commit** — `feat(resolve): structural dual-run event gate, arity-FP reconciled, unprojectable-fail (Phase 4b Task 4)`.

---

### Task 5: Independent event-route teeth + honest framing

**Files:** Modify `src/program/resolve/differential.rs`, `tests/program_resolve_harness.rs`, `CHANGELOG.md`; Test (fixture + CDO).

**Interfaces:** The false-edge check — INDEPENDENT of the index's own parse.

- [ ] **Step 1: Write failing tests** — (a) a fresh EventFlow edge whose publisher-anchored routine's stored subscribers include a subscriber whose **raw `[EventSubscriber]` AttributeIr** (re-read from the IR at gate time) does NOT name that publisher+event → `unverified_extra`; (b) a subscriber whose params do NOT structurally map to the publisher event params → `unverified_extra`; (c) a correct subscriber → passes. The check must NOT simply re-compare the edge to the `ParsedSubscriberArgs` that BUILT it (circular) — it re-derives from the raw IR / does the structural param-mapping.
- [ ] **Step 2: Run — fail.**
- [ ] **Step 3: Implement** the independent teeth: for each fresh EventFlow route, re-read the subscriber routine's raw `[EventSubscriber]` AttributeIr (via the ParsedUnit IR, not the index's cached parse) AND structurally verify the subscriber's parameter list is a valid prefix of the publisher event's signature. PASS → matched/fresh_only; FAIL → `unverified_extra`. Assert `unverified_extra == 0` (non-zero = a real false subscription edge — FIX). CHANGELOG: Phase 4b adds EventFlow Multicast edges — the publisher→subscriber CORE is dual-run-gated vs L3 (structural key, L3 arity-FPs reconciled, `l3_regression=0`); conditions/element/multiple-attrs are fixture-validated; InternalEvent non-shipping; Manual subscribers are conditional may-edges that default reachability does NOT traverse. NOT full event-modeling completion (table/page/database events, BindSubscription activation remain). Cite the gated core, not raw counts.
- [ ] **Step 4: Run WITH CDO_WS** — `l3_regression==0`, `unverified_extra==0`, `fresh_only_uncategorized==0`, deterministic; print the full breakdown.
- [ ] **Step 5: Full gate + commit** — `feat(resolve): independent event-route teeth + honest framing (Phase 4b Task 5)`.

---

## Roadmap — 1B.3 (next)

Full SymbolReference ABI cross-check (incl. cross-app event pub/sub); retire L3 oracle; the live edge-builder HonestEmpty/SetCompleteness wiring; **`BindSubscription` activation modeling (Manual reachability — turning the conditional may-edge into a precise bound-set)**; table/page/database trigger-events as EventFlow; element-filter route-matching; the carry-over receiver-gap buckets, same-arity-type overload disambig, `Insert(false)` run_trigger wiring, the 17 Cat-D divergences.

## Self-Review

- **Round-1 reviewer fixes incorporated:** (1) Manual subscriber gets a DISTINCT conditional obligation + `fires_by_default()` + the default-excludes-Manual reachability contract+test (Task 0) — not a reachability lie; (2) the gate reconciles L3's arity-blind false-positives via `l3_false_positive_arity_mismatch` so `l3_regression==0` is ACHIEVABLE given fresh's arity precision (Task 4) — not a mathematically-impossible gate; (3) publisher overload ambiguity → `unresolved_ambiguous`, never first-match (Task 2); (4) Task-5 teeth re-derive from RAW IR + structural param-mapping, INDEPENDENT of the index parse (Task 5) — not circular; (5) `RoutineNode` carries `Vec<ParsedSubscriberArgs>` (multiple `[EventSubscriber]`) (Task 2) — not Option; (6) the gate uses a fresh-OWNED STRUCTURAL key (publisher id+event+arity+subscriber), FAILS on unprojectable events, and machine-categorizes `fresh_only` with `fresh_only_uncategorized==0` (Task 4) — no opaque-hash silent-green, no free-form dumping ground; element filter has a home on `SubscriberEntry` (Task 2).
- **AlFile coupling** resolved by extract-time flat-parse (Task 2).
- **Spec coverage:** §5.3 → Tasks 1-5; §3.1 Multicast + the conditional-reachability refinement → Tasks 0,3; §6.5 gate → Task 4; §6.3 teeth → Task 5.
- **Placeholder scan:** the "read ExprKind / join PEventEdge↔PEventSymbol / re-read raw AttributeIr" steps name exact sources. No `TODO`.
- **Type consistency:** `ParsedSubscriberArgs`/`PublisherKind` (T1)→T2; `SubscriberEntry`/`ambiguous_subscriptions`/`subscribers_of` (T2)→T3,T4; `conditions: Vec`/`fires_by_default`/`ConditionalResolved` (T0)→T2,T3; `StructEventKey`/the projections (T4)→T5.
