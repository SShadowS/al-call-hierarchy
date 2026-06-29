# Plan 1B.2 Phase 3 — Member-Call Resolution (receiver lattice + member catalog) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve `Member` call sites (`Receiver.Method(...)`) by inferring the receiver's static type (a clean-room receiver-type lattice) and dispatching the method — emitting real evidenced routes — proven against L3 (the ORACLE) on Member sites. Opens with the overload discriminator that closes the 17 systematic Phase-2 divergences.

**Architecture:** Builds on Phase 2's `src/program/resolve/` resolver. Adds an arity discriminant to `RoutineNodeId` (Task 0), a `ReceiverType` lattice + Phase-A inference (`receiver.rs`), a clean-room member-builtin catalog (`member_catalog.rs`), and Phase-B dispatch staged by receiver type (Framework → Object → Record), gated incrementally vs L3. Interface/Enum fan-out is DEFERRED to Phase 4.

**Tech Stack:** Rust (edition 2024, toolchain 1.96.0), the `al-call-hierarchy` crate, `src/engine/l3` resolver as the read-only ORACLE.

**Source of truth:** `docs/superpowers/specs/2026-06-29-plan1b2-fresh-resolver-design.md` (§5.2 receiver lattice, §5.4 evidence, §6 gate). Read it first.

## Key facts grounding this plan (from the Phase-3 L3 receiver-lattice map)

- **Overload gap:** `RoutineNodeId{object, name_lc, enclosing_member_lc}` collapses AL overloads (same name, different arity) to one node. L3 disambiguates by PRIMARY arity (`m.parameters.len() == arg_count`, `call_resolver.rs:301-348`), SECONDARY arg-type (`disambiguate_by_arg_types`, `call_resolver.rs:267-296`). Adding `params_count: usize` to `RoutineNodeId` matches L3's primary key + closes the 17 Cat-D divergences (different-arity overloads in warehouse/email-template pages).
- **L3 `ReceiverType` lattice** (`receiver_type.rs:62-110`): `Object{kind,name}` / `Interface{name}` / `Enum{name}` / `Record{table_object_id}` / `SelfObject` / `RecordRef` / `FieldRef` / `KeyRef` / `Framework{ReceiverBuiltinKind}` / `Primitive` / `Dynamic` / `Unknown{reason}`.
- **Phase A** `infer_receiver_type` (`receiver_type.rs:165-426`): variable lookup (params→locals→object globals) → declared type text → `classify_receiver` (`member_builtins.rs:187-259`) maps type-text to a `ReceiverBuiltinKind`/Record/Object; singletons (CurrPage→`Framework{PageInstance}`, Session/NavApp/Database/etc.) hardcoded; `parse_object_type_ref` for `Codeunit X`/`Page X`/`Interface X`/`Enum X`. Fresh data: `RoutineDecl.params`/`locals` (each `ty: Option<String>`), `return_type`, `ObjectDecl.globals`.
- **Phase B** `dispatch` (`receiver_type.rs:997-1068`): per-variant. PHASE-3 arms (single-dispatch): `Record` (catalog-first, then table+extensions via `resolve_by_name_and_arity_multi`), `Object{Codeunit/Page/Report/Query/XmlPort}` (`dispatch_object`→resolve_by_name_and_arity; Codeunit.Run→OnRun), `SelfObject` (own object), `RecordRef`/`FieldRef`/`KeyRef`/`Framework{*}` (catalog-only `dispatch_framework`), `Enum`/`Primitive`/`Dynamic` (non-resolution). PHASE-4 (DEFER): `Interface` fan-out.
- **Member catalog:** `member_builtin_disposition(kind, method_lc) -> Option<Disposition{Builtin|FlowsType}>` (`member_builtins.rs:263-332`), 57 `ReceiverBuiltinKind`s. Authoritative source = `tools/gen-al-builtins/out/member_builtins.json` (97 type→methods entries). Clean-room: fresh catalog sources membership from that JSON, owns its disposition logic.
- **L3 member-edge dispatch kinds** (`receiver_type.rs`): `dispatch_object`/`dispatch_record`(non-builtin) → `DispatchKind::Method`; `dispatch_framework`/`dispatch_record`(catalog) → `DispatchKind::Builtin`; `<CU>.Run` → `CodeunitRun`. **Gate in-scope** = fresh `CalleeShape::Member` sites vs L3 edges from a `PCallee::Member` origin with `dispatch_kind ∈ {method, builtin, codeunit-run}`.
- **NOT Member-scope:** record-op calls (SetRange/FindSet/etc. on records) are `CalleeShape::RecordOp` → implicit-trigger (Phase 1/2). Member-scope Record calls = `Rec.UserDefinedProcedure()` (table procedures), not record-op builtins.
- **The 3397 `missing_site`** (Phase-2 deferred Member sites) breakdown estimate: Record table-procedures ~1500-2000 (the mass); Codeunit-var member calls ~800-1200; framework ~300-500; CurrPage compound ~100-200.

## Global Constraints

- Rust edition 2024; toolchain 1.96.0. `rustfmt <file>` per-file — **never** `cargo fmt`.
- Stage only files each task names — **never** `git add -A`.
- CI gates: `cargo clippy --release --all-features -- -D warnings` (NO `--tests`), `cargo fmt --check`, `cargo test --workspace`.
- **`src/engine/l3` + goldens are the read-only ORACLE — do not modify.** Do NOT import L3's `member_builtins.rs`/`receiver_type.rs` LOGIC into the fresh modules (clean-room: fresh owns its inference + dispatch + disposition; catalog MEMBERSHIP may source from the generator's `member_builtins.json`).
- Determinism: emitted collections sorted; harness run-to-run identical.
- CDO env-gated: `CDO_WS` (= `U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud`); tests return early when unset — run WITH it set.
- Update `CHANGELOG.md` per user-visible capability.
- **rust-analyzer diagnostics on `resolve/*` are NOT authoritative** (confirmed ~18×, incl. E0716/E0432 compile-error-class) — only `cargo build`/`test`/`clippy` count.

## File / module structure (this phase)

| File | Responsibility |
|------|----------------|
| `src/program/node.rs` (modify) | Task 0: `RoutineNodeId.params_count`. |
| `src/program/{node_extract.rs, resolve/stub.rs, resolve/body_map.rs, resolve/resolver.rs, resolve/index.rs}` (modify) | Task 0: populate `params_count` + arity-matched overload pick. |
| `src/program/resolve/receiver.rs` (create) | Task 1: `ReceiverType` lattice + `infer_receiver_type` (Phase A). |
| `src/program/resolve/member_catalog.rs` (create) | Task 2: clean-room member-builtin catalog (per-kind membership from `member_builtins.json`). |
| `src/program/resolve/resolver.rs` (extend) | Tasks 2-4: `resolve_member` dispatch (Framework → Object → Record). |
| `src/program/resolve/differential.rs` + `tests/program_resolve_harness.rs` (modify) | Task 5: Member-resolution gate. |

---

### Task 0: Overload discriminator — `RoutineNodeId.params_count`

**Files:**
- Modify: `src/program/node.rs`, `src/program/node_extract.rs`, `src/program/resolve/{stub.rs, body_map.rs, resolver.rs, index.rs}`
- Test: `src/program/resolve/resolver.rs` + the Phase-2 gate (the 17 divergences shrink)

**Interfaces:**
- Produces: `RoutineNodeId { object, name_lc, enclosing_member_lc, params_count: usize }`. Every constructor gains the field.

- [ ] **Step 1: Write the failing test** — a synthetic object with two same-named procedures of DIFFERENT arity (`Post()` and `Post(x: Integer)`); assert they produce DISTINCT `RoutineNodeId`s and `resolve_bare`/`resolve_in_object` picks the arity-matched one (call with arity 1 → the 1-param overload, NOT the 0-param).

```rust
#[test]
fn overloads_distinct_by_arity_and_resolved_by_arity() {
    // Build a synthetic graph+index+body_map with object C having Post() and Post(Integer).
    // Assert: the two RoutineNodeIds differ (params_count 0 vs 1).
    // Assert: resolve_bare(C, "post", arity=1, ...) resolves to the params_count==1 routine.
}
```

- [ ] **Step 2: Run to verify it fails** — `cargo test -p al-call-hierarchy program::resolve::resolver::tests::overloads_distinct_by_arity_and_resolved_by_arity`. Currently both overloads collapse to one `RoutineNodeId`.
- [ ] **Step 3: Implement** — add `pub params_count: usize` to `RoutineNodeId` (`node.rs:91-96`, keep all derives). Populate from `r.params.len()` at: `node_extract.rs:73-80`, `stub.rs:57-64`, `body_map.rs:57-68`. Build `--tests` to find every other construction site (the compiler lists them: `resolver.rs`/`index.rs`/`edge.rs` test helpers → `params_count: 0` for synthetics, or the right arity where the test cares). In `resolve_in_object` (`resolver.rs:140-199`): now that overloads are distinct `RoutineNodeId`s in `index.routines_in_object`, find the candidate whose `params_count == arity` (deterministic, lowest NodeId on tie). Remove the `candidates.len() > 1` first-candidate fallback's reliance on collapsed ids; if NO arity matches → Unknown (the Phase-2 Task-4 behavior). If MULTIPLE same-arity (overloads differing only by type) → first deterministically + a TODO noting `disambiguate_by_arg_types`-equivalent is deferred.
- [ ] **Step 4: Run to verify it passes** + run the Phase-2 gate WITH CDO (`CDO_WS=... cargo test -p al-call-hierarchy --test program_resolve_harness phase2 -- --nocapture`): confirm `regression_unexplained == 0` STILL holds and `divergence` DROPS (the 17 Cat-D should mostly resolve). Print the new divergence count; if some Cat-D remain (same-arity overloads), note them.
- [ ] **Step 5: Format, lint, full gate, commit**

```bash
rustfmt src/program/node.rs src/program/node_extract.rs src/program/resolve/stub.rs src/program/resolve/body_map.rs src/program/resolve/resolver.rs src/program/resolve/index.rs
cargo clippy --release --all-features -- -D warnings 2>&1 | tail -3
cargo test --workspace 2>&1 | grep -E "test result:|FAILED" | tail
git add src/program/node.rs src/program/node_extract.rs src/program/resolve/stub.rs src/program/resolve/body_map.rs src/program/resolve/resolver.rs src/program/resolve/index.rs
git commit -m "feat(program): RoutineNodeId arity discriminant — distinct overloads, closes Cat-D divergences (Phase 3 Task 0)"
```

---

### Task 1: Receiver-type lattice + Phase-A inference (`receiver.rs`)

**Files:**
- Create: `src/program/resolve/receiver.rs`
- Modify: `src/program/resolve/mod.rs`
- Test: `src/program/resolve/receiver.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces:
  - `pub enum ReceiverType { Object { kind: ObjectKind, name_lc: String }, Interface { name_lc: String }, EnumType { name_lc: String }, Record { table: Option<ObjectNodeId> }, SelfObject, RecordRef, FieldRef, KeyRef, Framework(FrameworkKind), Primitive, Dynamic, Unknown }` (clean-room; carries `ObjectNodeId`, not L3 strings).
  - `pub enum FrameworkKind { JsonObject, JsonToken, JsonArray, JsonValue, HttpClient, HttpRequestMessage, HttpResponseMessage, HttpContent, HttpHeaders, InStream, OutStream, TextBuilder, Dialog, List, Dictionary, Xml, PageInstance, ReportInstance, Session, NavApp, Database, IsolatedStorage, Text, /* … the high-volume subset; extend from member_builtins.json kinds as needed */ Other(String) }`.
  - `pub fn classify_type_text(ty: &str) -> ReceiverType` — maps a declared type string (`Record "Customer"`, `Codeunit 80`, `JsonObject`, `Integer`, `Interface IFoo`, `Variant`, …) to a `ReceiverType` (Record/Object carry the parsed name; framework kinds via the catalog kind table). Mirror the LOGIC of L3's `classify_receiver`/`parse_object_type_ref` (read `member_builtins.rs:187-259`, `receiver_type.rs:379-426`) but write it fresh.
  - `pub fn infer_receiver_type(receiver_lc: &str, routine: &RoutineDecl, object_globals: &[VarDecl], graph: &ProgramGraph, index: &ResolveIndex, from_object: &ObjectNode) -> ReceiverType` — Phase A: (1) singletons (`currpage`→`Framework(PageInstance)`, `currreport`→`Framework(ReportInstance)`, `session`/`navapp`/`database`/`isolatedstorage`/… hardcoded); (2) implicit `rec`/`xrec` → `Record{ from_object's implicit table }` (a Table object IS its own record; a TableExtension's is `extends_target`; a Page's source-table may be unavailable → `Record{None}`, note); (3) variable lookup `routine.params` then `routine.locals` then `object_globals` by name → `classify_type_text(ty)`; for `Record "X"`/`Codeunit X`/etc. resolve the name to an `ObjectNodeId` via `graph.resolve_object`/`index.object_by_number` (closure-scoped); (4) else → `Unknown`.

- [ ] **Step 1: Write failing tests** — `classify_type_text`: `Record "Customer"`→`Record`, `Codeunit 80`→`Object{Codeunit}`, `JsonObject`→`Framework(JsonObject)`, `Integer`→`Primitive`, `Variant`→`Dynamic`, `Interface "IFoo"`→`Interface`. `infer_receiver_type` over a synthetic routine with a `Cust: Record Customer` local + a `J: JsonObject` local + implicit `Rec`: assert each receiver name infers the right `ReceiverType` (incl. the Record's resolved table `ObjectNodeId`), `currpage`→`Framework(PageInstance)`, an unknown name→`Unknown`.
- [ ] **Step 2: Run to verify they fail.**
- [ ] **Step 3: Implement `receiver.rs`** — read `src/engine/l3/receiver_type.rs:165-426` + `member_builtins.rs:187-259` for the inference + classification LOGIC; write it fresh over the IR `RoutineDecl`/`VarDecl`/`Param` + `ProgramGraph`/`ResolveIndex`. Object-globals: thread them from the caller's `ObjectDecl.globals` (confirm how to reach the `ObjectDecl` from the resolver context — it's in the parsed `AlFile`; the resolver already has the parsed units / `BodyMap` — extend `BodyMap` to also expose the object's globals if needed, minimal).
- [ ] **Step 4: Run to verify they pass.**
- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/program/resolve/receiver.rs src/program/resolve/mod.rs
cargo clippy --release --all-features -- -D warnings 2>&1 | tail -3
git add src/program/resolve/receiver.rs src/program/resolve/mod.rs
git commit -m "feat(resolve): receiver-type lattice + Phase-A inference (Phase 3 Task 1)"
```

---

### Task 2: Framework/catalog member dispatch (lowest-risk, ~300-500 sites)

**Files:**
- Create: `src/program/resolve/member_catalog.rs`
- Modify: `src/program/resolve/resolver.rs`, `src/program/resolve/mod.rs`
- Test: both files (`#[cfg(test)]`)

**Interfaces:**
- `member_catalog.rs`: `pub fn member_builtin(kind: FrameworkKind_or_RecordRefFieldRefKeyRef, method_lc: &str) -> bool` (+ a `BuiltinId` accessor) — membership per receiver kind, sourced from `tools/gen-al-builtins/out/member_builtins.json` (document provenance + `catalog_version`). Clean-room: own disposition; data from the JSON.
- `resolver.rs`: `pub fn resolve_member(receiver: &ReceiverType, method_lc: &str, arity: usize, from_object: &ObjectNode, graph, index, body_map) -> (DispatchShape, Vec<Route>)` — THIS TASK implements ONLY the catalog arms: `RecordRef`/`FieldRef`/`KeyRef`/`Framework(_)` → `member_builtin(kind, method) ? Catalog route : Unknown`; `Record` catalog-first prefix (a Record method that's a member-builtin → Catalog) — but full Record table-proc dispatch is Task 4; for now Record-non-catalog → Unknown. `Object`/`SelfObject` → Unknown (Task 3). `Interface`/`EnumType` → Unknown (Phase 4). `Primitive`/`Dynamic`/`Unknown` → the honest non-resolution outcome (`Dynamic`→DynamicOpen).

- [ ] **Step 1: Write failing tests** — `member_catalog`: `JsonObject`.`add`/`get`→true, a non-method→false; `RecordRef`.`field`→true. `resolve_member`: a `Framework(JsonObject)` receiver + `add` → `Route{Builtin, Catalog, CatalogEntry}`; a `FieldRef` + `value` → Catalog; a `Primitive` + anything → Unknown; a `Dynamic` → `DynamicOpen`.
- [ ] **Step 2: Run to verify they fail.**
- [ ] **Step 3: Implement** `member_catalog.rs` (membership from the JSON — read `tools/gen-al-builtins/out/member_builtins.json` + `src/engine/l3/member_builtins.rs` for the kind set; build fresh `phf`/`HashSet` per kind) + the catalog arms of `resolve_member`.
- [ ] **Step 4: Run to verify they pass.**
- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/program/resolve/member_catalog.rs src/program/resolve/resolver.rs src/program/resolve/mod.rs
cargo clippy --release --all-features -- -D warnings 2>&1 | tail -3
git add src/program/resolve/member_catalog.rs src/program/resolve/resolver.rs src/program/resolve/mod.rs
git commit -m "feat(resolve): member-builtin catalog + Framework/RecordRef dispatch (Phase 3 Task 2)"
```

---

### Task 3: Object dispatch — Codeunit/Page/Report/SelfObject (~800-1200 sites)

**Files:**
- Modify: `src/program/resolve/resolver.rs`
- Test: `src/program/resolve/resolver.rs` (`#[cfg(test)]`)

**Interfaces:**
- Extend `resolve_member`: `ReceiverType::Object{kind, name_lc}` → `graph.resolve_object(from_object.id.app, kind, &name_lc)` (closure-scoped) → `resolve_in_object(target.id, method_lc, arity, index, body_map)` → `Route{Routine, tier_evidence, SourceSpan}` (or Unknown if object/method not found). Codeunit `.Run()`/`.RunModal()` with arity ≤1 → reuse the entry-trigger logic (OnRun). `SelfObject` → `resolve_in_object(from_object.id, ...)`.

- [ ] **Step 1: Write failing tests** — a `Cu: Codeunit "X"` receiver + `DoWork` (a proc on X) → `Route{Routine, Source}`; `Cu.Run()` → entry `OnRun` route; a `SelfObject` (`this.Helper()`) → own-object resolution; an Object receiver + nonexistent method → Unknown.
- [ ] **Step 2-4: TDD** as above.
- [ ] **Step 5: Commit** — `feat(resolve): Object/SelfObject member dispatch (Phase 3 Task 3)`.

---

### Task 4: Record dispatch — table procedures (the mass, ~1500-2000 sites)

**Files:**
- Modify: `src/program/resolve/resolver.rs`
- Test: `src/program/resolve/resolver.rs` (`#[cfg(test)]`)

**Interfaces:**
- Extend `resolve_member`: `ReceiverType::Record{table}`:
  1. `member_catalog`-first: a Record member-builtin → `Catalog` (already prefixed in Task 2 — keep).
  2. `table == None` → `Unknown` (table unresolved).
  3. `table == Some(table_id)` → `resolve_in_object(table_id, method_lc, arity)` UNION over `index.table_extensions_of(table_name_lc)` each `resolve_in_object` → the resolved table/extension procedure `Route{Routine, Source}`. (`DispatchShape::Exact` if one; the base+extension procs are a small set — for a NON-virtual table proc it's Exact; treat as Exact for Phase 3, the Multicast-over-extensions nuance is for triggers not procs.)

- [ ] **Step 1: Write failing tests** — a `Cust: Record Customer` receiver + `GetBalance` (a proc on table Customer) → `Route{Routine, Source}`; the same with a TableExtension adding a proc → resolves via the extension; a Record builtin (`Cust.Name`? no — a method like `Cust.GetView`) → Catalog; `table==None` → Unknown.
- [ ] **Step 2-4: TDD.**
- [ ] **Step 5: Commit** — `feat(resolve): Record table-procedure member dispatch (Phase 3 Task 4)`.

---

### Task 5: Phase-3 Member-resolution gate (dual-run vs L3)

**Files:**
- Modify: `src/program/resolve/differential.rs`, `tests/program_resolve_harness.rs`, `CHANGELOG.md`
- Test: `tests/program_resolve_harness.rs` (env-gated CDO)

**Interfaces:**
- Wire `resolve_member` into the harness for `CalleeShape::Member` workspace sites (using `infer_receiver_type` per site, anchored per-routine). Project resolved routes to canonical targets (same keying as Phase 2). Compare vs L3 edges from a `PCallee::Member` origin with `dispatch_kind ∈ {method, builtin, codeunit-run}`. Reuse the Phase-2 bucket engine (`regression_unexplained`/`evidence_overclaim`/`unverified_extra`/`verified_win`/`divergence`/`missing_site`) on the Member subset.
- The gate test `phase3_member_resolution_matches_or_beats_l3`: env-gated; assert `regression_unexplained == 0`, `evidence_overclaim == 0`, deterministic; print the categorized breakdown + the Member `missing_site` (the still-deferred residual — Interface fan-out [Phase 4], unresolved-table Records, compound CurrPage). Honest framing: state the paired-subset result + the residual, do NOT cite incomparable raw rates.

- [ ] **Step 1: Write the failing CDO gate test.**
- [ ] **Step 2: Run to verify it fails.**
- [ ] **Step 3: Implement** the Member-site harness path + bucketing. Run WITH `CDO_WS`; INVESTIGATE any `regression_unexplained`/`evidence_overclaim` > 0 (a receiver-inference gap, a dispatch precedence miss, a wrong evidence/witness) — fix, don't relax/hide. Categorize residual `missing_site` (Interface→Phase-4, table-None→Record-table-unresolved, compound→deferred). Adjudicate any new `divergence`.
- [ ] **Step 4: Run WITH CDO_WS** — print breakdown; confirm body ran; asserts pass; Member `missing_site` substantially below the Phase-2 3397.
- [ ] **Step 5: Full gate + commit** — `feat(resolve): Phase-3 Member-resolution gate vs L3 (Phase 3 Task 5)`.

---

## Roadmap — Phase 4 + 1B.3

Phase 4: `Interface`/`Enum` fan-out (`resolve_interface_dispatch` over the `implementers_of` index → `Polymorphic`/`Multicast` + `SetCompleteness::Partial` open-world), event-flow edges, and the `unverified_extra` gate gaining teeth (applicability, not just witness validity). 1B.3: full SymbolReference ABI cross-check (verify `Abi`/`Opaque` routes), deep re-baseline, retire the L3 oracle. Carry-over: same-arity-type-overload disambiguation (`disambiguate_by_arg_types`-equivalent); Page implicit-source-table identity in `ObjectNode`; the validate-field implicit-trigger field arg.

## Self-Review

- **Spec coverage:** §5.2 receiver lattice + Phase A/B → Tasks 1-4; §5.4 evidence (tier→Evidence reused) + §5.5 witness → Tasks 2-4; §5.6 member catalog → Task 2; §6 gate → Task 5; the Phase-2-review overload prereq → Task 0. Interface/Enum fan-out (§3.1 Polymorphic) correctly deferred to Phase 4.
- **Placeholder scan:** the "mirror L3's LOGIC by reading <file:line>, write fresh" steps are bounded (each names the exact L3 reference + the clean-room boundary). The receiver-lattice + catalog are large but staged; no `TODO`/`add appropriate X`.
- **Type consistency:** `RoutineNodeId.params_count` (Task 0) threads through all consumers; `ReceiverType`/`FrameworkKind`/`classify_type_text`/`infer_receiver_type` (Task 1) → Tasks 2-5; `member_builtin` (Task 2) → Tasks 2/4; `resolve_member` (Tasks 2-4) → Task 5; `Evidence`/`Witness`/`RouteTarget`/`DispatchShape`/`Edge` + the Phase-2 bucket engine reused.
- **Known follow-ups (Phase 4/1B.3):** Interface fan-out; same-arity-type overload disambiguation; Page source-table identity; ABI cross-check.
