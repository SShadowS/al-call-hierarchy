# Plan 1B.3a — ABI ingestion + L3-validated semantic golden + self-reported metric Implementation Plan

> Status: v2 — rewritten after gpt-5.5 GO-WITH-CHANGES + gemini-3.1-pro NO-GO. Convergent fatal flaws fixed:
> (1) the ABI "cross-check" was CIRCULAR (emission + verification both read the same ingested ABI) and used
> an under-specified `{Kind}::{name}` key → reframed as a structured-key INGESTION-INTEGRITY invariant
> (honest: proves the route maps back to raw ABI, NOT correctness vs the binary); (2) **the post-L3 golden
> must NOT be self-generated from the fresh engine's own blind output** (that cements its bugs as truth) —
> instead capture the **L3-validated per-edge target mapping** as a frozen Rust-owned SEMANTIC golden WHILE
> L3 is still present (this is the post-retirement correctness floor); (3) ingesting ABI launders
> "external" into "resolved" → split the Histogram into `resolved_source`/`resolved_catalog`/
> `resolved_abi_external`; (4) `RoutineNodeId{name_lc, params_count}` CLOBBERS same-arity-different-type
> overloads (BC base apps use them heavily) → a signature discriminator / collision bucket; (5) parsing
> Base Application's SymbolReference every build cycle → memory/time blowup → cached/`Arc` lazy ABI index;
> (6) the 1B.3b retirement guard expands beyond `abi_unverified==0` to the full contract floor.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Make the fresh engine self-sufficient for the L3 retirement that follows in 1B.3b — by (a) ingesting dependency `.app` SymbolReference ABI so cross-app calls/events to SymbolOnly deps resolve to honest `resolved_abi_external` routes; (b) capturing, WHILE L3 IS STILL PRESENT, the **L3-validated per-edge semantic golden** (the frozen target-mapping baseline that catches confidently-wrong edges after L3 is gone); (c) self-reporting the north-star real-`unknown` rate via an obligation-COVERAGE-based `Histogram` with an honest external/source taxonomy. Removes nothing — L3 + the dual-run gates stay green.

**Architecture:** Extends `src/program/`. Adds: a cached ABI-ingestion pass (structured keys, overload-safe) to `build_program_graph`; an ingestion-integrity invariant; an obligation inventory + `resolve_full_program`; the split Histogram taxonomy; and the L3-validated semantic edge golden (captured via the EXISTING L3 oracle, frozen as Rust-owned data). **L3 is USED here (to mint the golden) but not removed** — 1B.3b removes it, guarded on this plan's contract floor.

**Tech Stack:** Rust (edition 2024, toolchain 1.96.0). The SymbolReference PARSER (`src/engine/deps/symbol_reference.rs` `parse_symbol_reference` → `SymbolReferenceAbi`) is platform-DATA ingestion (like the builtin catalog), NOT L3 resolution logic — acceptable to consume from the fresh side. L3 (`src/engine/l3`) is consumed in Task 4 ONLY to MINT the validated golden (the last sanctioned oracle use before 1B.3b retires it).

**Source of truth:** `docs/superpowers/specs/2026-06-29-plan1b2-fresh-resolver-design.md` (§4 tiers/evidence, §5 resolution, §6 gate/contracts) + the moat metric (CLAUDE.md "The Moat": honest taxonomy resolved/builtin/dynamic/external/unknown). Read first.

## Key facts grounding this plan (from the 1B.3 architectural map)

- **The gap:** true SymbolOnly deps (`source=None`, `snapshot/parse.rs:63`) get NO graph nodes → cross-app calls → `Unknown`; dep event publishers not emitted. EmbeddedSource deps ARE in-graph.
- **The ABI data:** `AppUnit.abi: Option<ParsedAppPackage>` (legacy, NO arity). The RICHER `SymbolReferenceAbi` (`engine/deps/symbol_reference.rs:122`, `parse_symbol_reference:688`) HAS arity: `AbiObject { object_type, object_number, name, routines, implemented_interfaces, extends_target_name, source_table_name }`, `AbiRoutine { name, kind, event_kind, parameters: Vec<AbiParameter{name,type_text,is_var}>, is_local, is_internal }`. BC 24+ namespace recursion handled (`collect_raw_objects`). **Ingest the RICHER `SymbolReferenceAbi`** (arity + param TYPES needed — see overload fix).
- **The Abi/Opaque routes** (`edge.rs`): `RouteTarget::AbiSymbol { app: AppRef, symbol_key: String }`, `Witness::AbiSymbol`, `Evidence::{Abi, Opaque}`. The CURRENT `symbol_key = format!("{:?}::{}", kind, name_lc)` is UNDER-SPECIFIED (no object name/number/arity) — Task 1 replaces it with a structured key BEFORE any golden freezes the weak shape.
- **`classify_obligation`/`real_unknown_rate`/`Histogram::of_edges`** (`edge.rs:221/262/297`) L3-INDEPENDENT (Resolved/ConditionalResolved/HonestDynamic/HonestEmpty/Unknown) but `Resolved` does NOT distinguish source/catalog/external, and the metric is over EMITTED edges (can't see missed sites). The cross-app primary-scope model is in `aldump --l3-call-graph-stats-cross-app`.
- **Contracts surviving L3:** `witness_contract_holds` (`differential.rs:1116`), `verify_event_subscriber_route` (`differential.rs:3149`). **The L3 oracle** (`differential.rs` `project_l3*`) is the ONLY current catcher of confidently-wrong target_ids — Task 4 freezes its per-edge verdicts before it's retired.

## Global Constraints

- Rust edition 2024; toolchain 1.96.0. `rustfmt <file>` per-file — never `cargo fmt`. Stage only named files — never `git add -A`.
- CI gates: `cargo clippy --release --all-features -- -D warnings` (NO `--tests`), `cargo fmt --check`, `cargo test --workspace`. All pass.
- **REMOVES NOTHING.** L3 + ALL existing dual-run gates stay intact + GREEN. Purely additive. (1B.3b retires L3 — separate plan.)
- **The ABI check is an INTEGRITY invariant, NOT a correctness verifier** — it proves a route maps back to the raw SymbolReference (no mangling), NOT that the route is the semantically-correct target. Frame it honestly; do NOT call it "verification."
- **Do NOT self-generate the correctness golden** — the semantic baseline (Task 4) is the L3-VALIDATED per-edge target mapping (captured via the L3 oracle while present), NOT the fresh engine's own blind output. Counts/histograms are telemetry, not correctness proof.
- **No laundering:** `resolved_abi_external` is a DISTINCT bucket from `resolved_source`/`resolved_catalog`. The real-unknown rate drop from ABI ingestion is reported as "moved to external," not "resolved more."
- **Overload-safe ingestion:** same-arity-different-type ABI overloads MUST NOT clobber each other in the graph.
- **Bounded cost:** the ABI index is parsed once + cached (`Arc`), not re-allocated per build cycle.
- Determinism; CDO env-gated (`CDO_WS`); CHANGELOG honest. rust-analyzer diagnostics on resolve/* NOT authoritative — only cargo.

## File / module structure

| File | Responsibility |
|------|----------------|
| `src/program/abi_ingest.rs` (create) | Task 1: cached parse → SymbolOnly nodes (structured key, overload-safe, version-distinct). |
| `src/program/node.rs` / `edge.rs` (modify) | Task 1: structured `AbiRoutineKey` on `RouteTarget::AbiSymbol`; the overload signature discriminator. |
| `src/program/build.rs` (modify) | Task 1: call the cached ABI-ingestion pass. |
| `src/program/resolve/abi_check.rs` (create) | Task 2: `abi_ingestion_integrity(edges, raw_abi_index)` (structured-key membership vs RAW SymbolReference). |
| `src/program/resolve/edge.rs` (modify) | Task 2-3: Histogram taxonomy split (`resolved_source`/`resolved_catalog`/`resolved_abi_external`). |
| `src/program/resolve/full.rs` (create) | Task 3: obligation inventory + `resolve_full_program`. |
| `src/bin/aldump.rs` (modify) | Task 3: `--program-call-graph-stats` → fresh taxonomy'd Histogram + coverage. |
| `src/program/resolve/semantic_golden.rs` (create) + `tests/goldens/` | Task 4: capture + assert the L3-validated per-edge target golden. |
| `tests/program_resolve_harness.rs` (modify) | Tasks 1-4: gates + contracts + goldens. |

---

### Task 1: ABI ingestion (structured key, overload-safe, cached)

**Files:** Create `src/program/abi_ingest.rs`; modify `src/program/node.rs` + `edge.rs` (structured key + overload discriminator), `build.rs`; Test (unit + SymbolOnly-dep fixture).

**Interfaces:** `ingest_abi(unit, app, abi_cache) -> (Vec<ObjectNode>, Vec<RoutineNode>)`. New `AbiRoutineKey { app: AppRef, object_type: String, object_number: i64, object_name_lc: String, routine_name_lc: String, params_count: usize, param_type_fp: u64, routine_kind: AbiRoutineKind, event_kind: AbiEventKind }` (carry `event_kind` EXPLICITLY — Integration/Business/Internal/None — don't fold it into `routine_kind`) carried on `RouteTarget::AbiSymbol` (replaces the stringly `symbol_key`). The overload discriminator: extend `RoutineNodeId` (or the ABI node id) with a `sig_fp` (= `param_type_fp`) so same-arity-different-type overloads are DISTINCT nodes (not clobbered).
**DETERMINISTIC overload-dispatch fallback (reviewer-mandated):** dispatch (picking the right overload at a call site by arg TYPES) is future work — but the INTERIM behavior MUST be stable, else CDO edge tests flap on hash-iteration randomness. When a call resolves to MULTIPLE same-arity candidates differing only by `param_type_fp`: deterministically pick the candidate with the lowest `param_type_fp` (a total order) AND tag the route so it's auditable, OR emit a single `AmbiguousOverload`-marked route — never a hash-order `.first()`. Document the choice; it must be referentially stable run-to-run.

- [ ] **Step 1: Write failing tests** — a SymbolOnly dep declaring Codeunit "Dep Pub" `[IntegrationEvent] OnDepEvent(p1: Integer, p2: Text)` + `procedure DoDepWork(x: Integer)` + an OVERLOAD pair `procedure F(a: Integer)` / `procedure F(a: Text)` → after `build_program_graph`: `ObjectNode{Codeunit, "Dep Pub", SymbolOnly}` + a `RoutineNode` for OnDepEvent (params_count 2, publisher_kind Some(Integration)) + DoDepWork + **BOTH `F` overloads present as DISTINCT nodes** (the same-arity-type-overload clobber fix); workspace-only snapshot → graph unchanged; an EmbeddedSource dep → NOT double-ingested; the ABI parse is cached (a second `build_program_graph` on the same snapshot reuses the `Arc`, no re-parse).
- [ ] **Step 2: Run — fail.**
- [ ] **Step 3: Implement** `abi_ingest.rs`: for each SymbolOnly dep `AppUnit`, parse its `SymbolReference.json` → `SymbolReferenceAbi` via a CACHED `Arc<SymbolReferenceAbi>` keyed by (app guid, version) (parse once; AppRef distinguishes dep versions). For each `AbiObject` → `ObjectNode{ SymbolOnly, implements, extends, source_table }`; for each `AbiRoutine` → a `RoutineNode` with the structured key + `param_type_fp` (a hash of the param `type_text`s) so overloads don't clobber; set `publisher_kind` from kind/event_kind; SKIP `is_local`/`is_internal` routines not callable from the workspace app. Replace the `RouteTarget::AbiSymbol.symbol_key` String with the structured `AbiRoutineKey`. In `build.rs`, call ingestion for SymbolOnly deps + re-sort nodes. Confirm SymbolOnly nodes flow into `ResolveIndex` (so `resolve_object`/`routines_in_object` find them → AbiSymbol routes).
- [ ] **Step 4: Run — pass.** Existing tests green (workspace graph undisturbed; the structured-key migration updates all AbiSymbol constructors).
- [ ] **Step 5: rustfmt + clippy + `cargo test --workspace` + commit** — `feat(program): cached overload-safe ABI ingestion + structured AbiRoutineKey (1B.3a Task 1)`.

---

### Task 2: ABI ingestion-integrity invariant + Histogram taxonomy split

**Files:** Create `src/program/resolve/abi_check.rs`; modify `src/program/resolve/edge.rs` (Histogram); Test (fixture + env-gated CDO).

**Interfaces:** `abi_ingestion_integrity(edges, raw_abi_index) -> AbiIntegrityReport { abi_routes_total, abi_mapped, abi_unmapped, abi_unmapped_sites }` — for every `AbiSymbol` route, the structured `AbiRoutineKey` resolves to a real entry in an index built FROM THE RAW SymbolReference (re-parsed, independent of the ingested `ProgramGraph` nodes). Honest framing: this proves the route MAPS BACK to raw ABI (no ingestion mangling), NOT semantic correctness. Plus the `Histogram` gains `resolved_source`/`resolved_catalog`/`resolved_abi_external` (splitting the old `resolved`).

- [ ] **Step 1: Write failing tests** — (fixture) the Task-1 dep graph + a workspace caller `"Dep Pub".DoDepWork(x)` → an `AbiSymbol` route → `abi_ingestion_integrity` maps it (raw-ABI index has Codeunit "Dep Pub".dodepwork/arity 1) → `abi_mapped+=1, abi_unmapped=0`; a fabricated `AbiSymbol` route whose `AbiRoutineKey` names a non-existent routine → `abi_unmapped+=1`; the Histogram splits: a Source route → `resolved_source`, a Catalog route → `resolved_catalog`, an AbiSymbol route → `resolved_abi_external` (NOT lumped into one `resolved`). (env-gated CDO) `abi_ingestion_integrity` over the full edge set → `abi_unmapped == 0` (every AbiSymbol route maps to a raw-ABI symbol — a miss = an ingestion/key-derivation bug, FIX not relax), deterministic; print `abi_routes_total`/`abi_mapped`.
- [ ] **Step 2: Run — fail.**
- [ ] **Step 3: Implement** `abi_check.rs`: build the raw-ABI index `AbiRoutineKey-shaped → exists` from a FRESH parse of the deps' SymbolReference (independent of the graph nodes — the integrity point). Map every `AbiSymbol` route's structured key. `abi_unmapped` records misses. Split `Histogram` per evidence (Source→resolved_source, Catalog→resolved_catalog, Abi/Opaque→resolved_abi_external; the real-unknown rate = unknown/total UNCHANGED, but the resolved breakdown is now honest).
- [ ] **Step 4: Run WITH CDO_WS** — `abi_unmapped == 0`; print the taxonomy'd histogram + abi coverage. Confirm body ran.
- [ ] **Step 5: Full gate + commit** — `feat(resolve): ABI ingestion-integrity invariant + Histogram source/catalog/external split (1B.3a Task 2)`.

---

### Task 3: Obligation inventory + `resolve_full_program` + self-reported metric

**Files:** Create `src/program/resolve/full.rs`; modify `src/bin/aldump.rs`; Test (CDO).

**Interfaces:** `obligation_inventory(graph, index, body_map) -> Vec<Obligation>` (EVERY parsed call/event site, regardless of outcome — each with a stable `ObligationId` = site identity) + `resolve_full_program(snapshot) -> (Vec<Edge>, Coverage)` where each `Edge` carries its `obligation_id`. The COVERAGE contract is **distinct-id SET equality** (reviewer-mandated — NOT count equality, which lies under multi-target/conditional/duplicate emission): `set(obligation_inventory.ids) == set(edges.obligation_id)`. Unknown/HonestDynamic/HonestEmpty are VALID classifications (a stable edge/record per obligation, possibly empty/opaque routes) — "silently absent" is the only failure. Catches MISSED sites a histogram-over-emitted-edges cannot.

- [ ] **Step 1: Write the failing tests** — (CDO env-gated) `resolve_full_program` over CDO → `parsed_obligations == classified_edges` (the coverage contract — no obligation silently dropped); `Histogram::of_edges` yields the taxonomy'd breakdown + real-unknown rate; `evidence_overclaim == 0`; `abi_unmapped == 0`; deterministic. Record the CDO real-unknown rate + the resolved_source/catalog/external split (Rust-owned telemetry baseline). Assert the rate ≤ a recorded ceiling.
- [ ] **Step 2: Run — fail.**
- [ ] **Step 3: Implement** `full.rs`: `obligation_inventory` enumerates every `CalleeShape` site + every publisher (the DENOMINATOR — defined over PARSED obligations, NOT emitted edges); `resolve_full_program` resolves each → exactly one `Edge` (bare/member/implicit-trigger/event), asserting coverage. Cross-app primary scope helper (mirror `--l3-call-graph-stats-cross-app`). Wire `--program-call-graph-stats` → the taxonomy'd Histogram (total/resolved_source/resolved_catalog/resolved_abi_external/conditional_resolved/honest_dynamic/honest_empty/unknown + real_unknown_rate) + coverage + abi summary. (Keep `--l3-call-graph-stats` — 1B.3b removes it.)
- [ ] **Step 4: Run WITH CDO_WS** — coverage holds (parsed==classified); the self-reported taxonomy'd rate prints; `evidence_overclaim==0`, `abi_unmapped==0`, deterministic. RECORD the rate + split.
- [ ] **Step 5: Full gate + commit** — `feat(resolve): obligation-coverage inventory + resolve_full_program + taxonomy'd self-reported metric (1B.3a Task 3)`.

---

### Task 4: L3-validated semantic edge golden + route-applicability contract

**Files:** Create `src/program/resolve/semantic_golden.rs` + `tests/goldens/`; modify `tests/program_resolve_harness.rs`, `CHANGELOG.md`; Test (CDO-mint + fixture-assert).

**Interfaces:** The post-L3 correctness FLOOR — captured from L3 WHILE PRESENT (NOT self-generated). `mint_l3_validated_golden(workspace) -> SemanticGolden` (per-edge: site → the L3-VALIDATED target id(s)); `assert_against_semantic_golden(fresh_edges, golden)` (fresh's per-edge target id MATCHES the L3-validated target — catches confidently-wrong edges post-L3). Plus a `route_applicability` contract.

- [ ] **Step 1: Write the failing tests** — (a) **the in-repo semantic golden (the CI floor):** for the in-repo fixtures (committable, deterministic — overloaded procs, same-name-different-object, member dispatch, SymbolOnly ABI calls, cross-app events, must-stay-Unknown negatives, must-stay-HonestDynamic), MINT the golden from L3's resolved per-edge target ids (the L3 oracle is still present), commit it (`tests/goldens/semantic-edges/`), and assert `resolve_full_program`'s per-edge target ids MATCH it (NOT just counts — the target IDENTITY, the confidently-wrong-edge catcher). (b) **the CDO/L3 semantic AUDIT (the real-world net — reviewer-mandated, since synthetic fixtures don't replicate Base-App namespace shadowing / overload collisions):** env-gated (`CDO_WS`), WHILE L3 is present, compute the per-edge L3-vs-fresh target-id diff over CDO's cross-app + member edges; the test asserts NO `confidently_wrong` class (fresh emits a non-Unknown target ≠ L3's validated target) goes UNREDUCED — every novel confidently-wrong class found MUST be reduced to a minimal in-repo fixture (added to (a)). Capture a deterministic CDO edge-mapping sample as an env-gated golden (the real-world shield; not committed as proprietary data — a hash/digest is committable). (c) **route-applicability contract** (L3-independent): every Source route's target is in-scope for the obligation (object/member/arity); Catalog → real catalog entry + arity-applicable; AbiSymbol → key in the raw ABI; event → subscriber attribute names that publisher. (d) the structural contracts (witness↔evidence, no route both Catalog and Source).
- [ ] **Step 2: Run — fail / mint the golden** (capture from L3) — inspect it is sane (the L3-validated targets are correct on the fixtures; this is the LAST sanctioned L3 use).
- [ ] **Step 3: Implement** `semantic_golden.rs` (the minting from L3 + the per-edge target-id assertion) + the `route_applicability` contract; commit the golden data. CHANGELOG: 1B.3a — the fresh engine ingests dependency SymbolReference ABI (cross-app calls/events → honest `resolved_abi_external` routes, integrity-checked `abi_unmapped=0`), self-reports the north-star real-`unknown` rate via an obligation-COVERAGE `Histogram` with an honest source/catalog/external taxonomy (the rate drop from ABI ingestion is "moved to external," NOT "resolved more"), and — critically — CAPTURES the **L3-validated per-edge semantic golden** as the frozen correctness floor that survives L3's retirement. This is the prerequisite for 1B.3b. State honestly: the ABI check is an INTEGRITY invariant not a binary verifier; the semantic correctness baseline is L3-minted (not self-generated). L3 + the dual-run gates remain in place + green.
- [ ] **Step 4: Run** the golden + applicability + contract tests + (WITH CDO_WS) the full suite — all green.
- [ ] **Step 5: Full gate + commit** — `feat(resolve): L3-validated semantic edge golden + route-applicability contract (1B.3a Task 4)`.

---

## Roadmap — 1B.3b (next) + the EXPANDED retirement guard

**1B.3b — retire the L3 oracle** (only after 1B.3a green): remove `project_l3*` + `assemble_and_resolve_workspace_default` from `differential.rs`; gut the L3-oracle internals of the harnesses, keeping the L3-INDEPENDENT floor. Keep `src/engine/l3` intact (the aldump L3→L4→L5→gate backbone).
**The EXPANDED guard (reviewer-mandated — `abi_unmapped==0` ALONE is insufficient): 1B.3b MUST NOT start until ALL of — `abi_unmapped==0` (structured key, raw index); the L3-validated SEMANTIC edge golden captured + green; the obligation-COVERAGE contract green (distinct-id set equality, no missed sites); the route-applicability contract green; the complete edge-set fixture goldens green (per-edge target identities, not counts); the self-reported real-unknown histogram stable ≤ ceiling; the existing L3 dual-run gates still green throughout 1B.3a; AND the CDO/L3 semantic AUDIT produces NO unreduced confidently-wrong divergence class (every one reduced to a committed in-repo fixture); AND the interim overload-dispatch fallback is deterministic (no hash-order flap).** Otherwise L3 retires while confidently-wrong source/member/event targets — especially Base-App namespace-shadowing / overload-collision ones that only appear at real-world density — remain uncatchable.

**Beyond:** cross-app event pairs now ABI-resolved; BindSubscription activation; table/page/database trigger-events; element-filter matching; the same-arity-type overload DISPATCH disambiguation (Task 1 makes them distinct NODES; resolving the right one at a call site is still future); the receiver-gap buckets, 17 Cat-D.

## Self-Review

- **Round-1 reviewer fixes incorporated:** (1) the ABI check is an honest INTEGRITY invariant with a STRUCTURED key vs the RAW SymbolReference (not the circular `{Kind}::{name}` self-check) — Task 2; (2) the post-L3 correctness golden is L3-VALIDATED (minted from the oracle while present), NOT self-generated from blind output — Task 4 (the highest-leverage fix); (3) the Histogram splits `resolved_source`/`catalog`/`abi_external` so ABI ingestion isn't laundered as "resolved more" — Tasks 2-3; (4) same-arity-different-type overloads get a `param_type_fp` discriminator (no clobber) — Task 1; (5) cached `Arc<SymbolReferenceAbi>` (no per-cycle Base-App re-parse) + version-distinct AppRef — Task 1; (6) the obligation-COVERAGE contract (parsed==classified) catches missed sites the histogram can't — Task 3; (7) the route-applicability contract — Task 4; (8) the 1B.3b guard expanded to the full floor.
- **Decomposition honored:** additive, removes nothing; 1B.3b retires L3 after the full floor is green. L3 is USED (Task 4) only to mint the golden — the last sanctioned oracle use.
- **Spec coverage:** §4 tiers (SymbolOnly ABI) → T1; §6 contracts (integrity, applicability, semantic golden, evidence_overclaim) → T2,T4; the moat metric (honest taxonomy, coverage) → T3.
- **Placeholder scan:** "parse SymbolReference / mint from L3 / mirror cross-app scope" name exact sources. No `TODO`.
- **Type consistency:** `AbiRoutineKey`/`ingest_abi` (T1) → `abi_ingestion_integrity` (T2) → `resolve_full_program` (T3) → the semantic golden (T4); the split `Histogram` (T2) → T3,T4.
