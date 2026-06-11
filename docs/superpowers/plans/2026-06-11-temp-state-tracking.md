# Temp-State Tracking Epoch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the `TempState = Known(bool) | ParameterDependent(i) | Unknown` substrate in the Rust AL engine so temp-record knowledge is captured at every declaration site (member/global vars, `TableType=Temporary` tables, `SourceTableTemporary` pages, ABI params), substituted soundly through the call graph (PD at composition), and resolved exactly per evidence path — suppressing false criticals on in-memory buffers while every uncertain case keeps firing.

**Architecture:** Layered substrate fix, detectors stay near-pure consumers. L0/L2 capture (structural `temporary_keyword`/property reads) → L3 resolution (table-override → var-decl → PD/Unknown, first-wins shadowing) → L4 composition (PD substituted per-callsite at effect inheritance) → L5 path-walk (PD resolved frame-by-frame to final severity). One real detector policy change: the RV-1 CalcFields/FlowField gate in d1.

**Tech Stack:** Rust, tree-sitter-al V2, salsa, rayon. Golden differential tests under `tests/`. Implementation lives ONLY in `src/engine/` (post-flip, Rust-only).

---

## Implementation source-of-truth

Spec: `docs/superpowers/specs/2026-06-11-temp-state-tracking-design.md` — implement to **Revision 2 (RV-1..RV-10)** + "Implementation & baseline plan (post-flip)". The spec is 3-reviewer-vetted + empirically verified — DO NOT re-litigate its decisions.

## Governing soundness rule (applies to EVERY task)

**Suppression-direction asymmetry:** every new `Known(true)` source must be syntax/property-exact (AST `temporary_keyword` node or object property — NEVER string-sniffing `\btemporary\b` over type text, which matches inside quoted table names). Every uncertainty path → `Unknown` → fires. Suppression is gated exclusively on `Known(true)`. The metamorphic oracle (Task 17) mechanically enforces this.

## House rules (apply to EVERY task)

- `rustfmt <changed-files>` per-file — NEVER `cargo fmt` (whole-crate churn).
- Full `cargo test` green after every task. If build dies with LNK errors → U: disk full → `cargo clean` + retry.
- Stage only intended paths — NEVER `git add -A`.
- Engine never throws; every capture/parse failure → field absent → `Unknown` → conservative firing.
- Never touch LSP shipping code beyond additive (`src/engine/*`, additive `lib.rs`/`Cargo.toml` only).
- `KNOWN_DIVERGENCES.json` stays `[]` permanently.
- Update `CHANGELOG.md` (per CLAUDE.md) — group under Added/Changed/Fixed.

## Verified current-state map (file:line — trust these, verified this epoch)

- `src/engine/l2/scope.rs:242-277` — `extract_object_globals` → `Vec<PVariableSymbol>` (no temp_state).
- `src/engine/l2/scope.rs:136-138` (params) & `184-186` (locals) — the keyword-read: `named_children(record_type_node).iter().any(|c| c.kind() == "temporary_keyword")`.
- `src/engine/l2/features.rs:61-72` — `PTempState { kind: String, value: Option<bool>, parameter_index: Option<u32> }`.
- `src/engine/l2/features.rs:219-231` — `PRecordVariable { id, name, table_name, temp_state, is_parameter, parameter_index }` — NO `scope`.
- `src/engine/l2/features.rs:102-128` — `PCallArgumentBinding` carries `source_parameter_index`, `caller_source_parameter_is_var`, `source_temp_state: Option<PTempState>`.
- `src/engine/l2/features.rs:191-208` — `PRecordOperation` has `temp_state: PTempState`, `field_arguments`, `field_argument_infos`.
- `src/engine/l2/mod.rs:346-360` — op-backfill copies rec-var temp_state onto each op by lc name.
- `src/engine/l3/record_types.rs:8` — self-documented "LAST-wins" comment; `variable_decl_by_name` built last-wins (~:112-114); op tableId resolution passes ~:87-173; implicit Rec/xRec ~:131-173.
- `src/engine/l3/l3_workspace.rs:840-852` — per-routine `record_variables` built from L2 features; globals (`extract_object_globals`, ~:780) NOT promoted. `L3RecordVariable` ~:124-158 (has `temp_state`, `temp_state_known_value()`), `L3RecordOperation` ~:160-188 (`temp_state: Option<PTempState>`). `L3Table` ~:112-122 (no `is_temporary`). `L3Object` ~:36-79 (`source_table_name`, no `SourceTableTemporary`). `classify_field` ~:453-485. `index_table` ~:487-596. Page source_table parse ~:699-703.
- `src/engine/l4/summary_runner.rs:349-635` — `compose_routine`; edge-inheritance loop ~:380-459; arg-bindings loop ~:502-587; `MAX_FIXED_POINT_ITERATIONS=1000` (:34, convergence cap).
- `src/engine/l4/effect_lattice.rs:76-97` — `TempStateKind { Known(bool), ParameterDependent(u32), Unknown }` + `key_fragment()`; `effect_key_of` ~:122-135 (temp_state in key); `via_for_edge_kind` ~:173-179.
- `src/engine/l4/combined_graph.rs:38-50` — `CombinedEdge { from, to, kind, callsite_id: Option<String>, operation_id, event_id, ... }`; event-dispatch edges have `callsite_id: None` (~:304-331).
- `src/engine/l3/call_resolver.rs:89-96` — `UpgradedBinding { parameter_index, callee_parameter_is_var, binding_resolution }`.
- `src/engine/l5/path_walker.rs:54-145` — `WalkResult { path: Vec<EvidenceStep>, ... }`, `walk_evidence(...)`, `WalkPolicy` trait, `Terminal { routine_id, local_loop_depth, op_id }`.
- `src/engine/l5/finding.rs:73-80` — `EvidenceStep { routine_id, operation_id, callsite_id, loop_id, source_anchor, note }`.
- `src/engine/l5/detectors/d1.rs:69,75,172-207,254-260` — `is_known_temp`, `is_temp_uncertain`, severity policy, temp notes; `merge_by_terminal` from `path_merge.rs:188-251`.
- `src/engine/l5/detectors/d33.rs:54-59,70-75` — param-skip + temp suppression; intra-routine.
- temp gates: d3 (`d3.rs:167` via `temp_state_known_value`), d18 (`78-81`), d36 (`67-72`), d37 (`79-84`), d40 (`116-121`, on binding). d5/d10/d49 do NOT reference temp_state.
- `src/engine/deps/symbol_reference.rs:41-47` `AbiParameter { name, type_text, is_var }`; `:140-144` `RawTypeDef { name }` (NO Temporary); `:146-154` `RawParam`; `:49-65` `AbiRoutine` (NO record vars); tables `:720-754`; `parse_field` `:545-576` (field_class/is_blob_like).
- `src/engine/gate/cache_prune.rs:46` — `CACHE_VERSION_SYMBOL_READER: &str = "17"`. Test `tests/cli_c_cache_differential.rs:354,502,532`.
- `src/engine/l5/digest.rs:1911-2037` — `SnapTempState` merge, "stays known-temp only if BOTH are" (physical-if-ANY) — PRESERVE.
- Golden harness `tests/differential.rs`; AL_SEM_DIR `#[ignore]` refresh `tests/r2_5b_refresh.rs`. Goldens are Rust-owned baselines in `tests/*-goldens/`.

---

## Task 0: RV-5 — flip last-wins → first-wins shadowing (PREREQUISITE)

**Why first:** once tempState backfill lands in `record_types.rs`, a temp GLOBAL shadowing a non-temp LOCAL would stamp `Known(true)` on the local's ops → silent suppression of a real finding. Must flip the name-map collision rule to innermost-wins BEFORE any tempState logic touches that file.

**Files:**
- Modify: `src/engine/l3/record_types.rs` (the `variable_decl_by_name` build, ~:112-114, and line 8 doc comment)
- Test: `tests/temp_state_shadowing.rs` (Create)

- [ ] **Step 1: Write the failing test.** Create `tests/temp_state_shadowing.rs`. Build a minimal in-repo fixture (follow `tests/r0-corpus/` workspace shape — `app.json` + `src/*.al`) where a codeunit has a GLOBAL record var `Foo: Record "Bar";` (physical) and a procedure declares a LOCAL `Foo: Record "Baz";` of a DIFFERENT table, with a record op on `Foo`. Assert (via the L3 resolution entry / `assemble_and_resolve_workspace` used by other tests) that the op's resolved `table_id` matches the LOCAL's table (`Baz`), not the global's (`Bar`). Use the same harness entry the differential tests use to obtain L3 routines.

- [ ] **Step 2: Run test, verify it fails.** Run: `cargo test --test temp_state_shadowing -- --nocapture`. Expected: FAIL — op resolves to the global's table (last-wins bug).

- [ ] **Step 3: Implement first-wins.** In `record_types.rs`, change the `variable_decl_by_name` insertion from unconditional `.insert(k, v)` (last-wins over the params→locals→globals-ordered `routine.variables`) to first-wins: `variable_decl_by_name.entry(k).or_insert(v)` (or `if !map.contains_key(&k) { map.insert(k, v); }`). Since `routine.variables` is ordered params→locals→globals, first-wins makes the innermost (param/local) declaration win. Update the line-8 doc comment from "LAST-wins" to "FIRST-wins (innermost declaration wins on a name collision)".

- [ ] **Step 4: Run test, verify pass.** Run: `cargo test --test temp_state_shadowing -- --nocapture`. Expected: PASS.

- [ ] **Step 5: Full suite.** Run: `cargo test`. Expected: GREEN. (Goldens may already encode first-wins correctly; if any R2a tableId golden moves, note it for the Task 19 rebaseline review — but a pure-physical fixture should not move existing goldens.)

- [ ] **Step 6: rustfmt + commit.** `rustfmt src/engine/l3/record_types.rs tests/temp_state_shadowing.rs`. Stage only those two files + CHANGELOG.md. Commit: `fix(engine-ts0): flip record_types name-map to first-wins shadowing (RV-5 prerequisite)`.

---

## Task 1: model fields — `scope` on record vars + `is_temporary` on tables + page temp flag

Pure additive model changes (no behavior yet) so later tasks have somewhere to write. All new fields optional/defaulted so serialization stays shape-stable where possible (note: new serialized fields WILL move goldens — that is the Task 19 rebaseline's job; keep them `skip_serializing_if`/`Option` to minimize churn).

**Files:**
- Modify: `src/engine/l2/features.rs` (`PRecordVariable` +`scope`)
- Modify: `src/engine/l3/l3_workspace.rs` (`L3RecordVariable` +`scope`; `L3Table` +`is_temporary`; `L3Object` +`source_table_temporary`)
- Modify: `src/engine/deps/symbol_reference.rs` (`AbiTable` +`is_temporary`; `AbiField` unchanged; `AbiParameter` +`is_temporary`)
- Test: covered by `cargo build` + existing tests compiling.

- [ ] **Step 1: Add `scope` to `PRecordVariable`.** In `features.rs:219-231` add `#[serde(skip_serializing_if = "Option::is_none")] pub scope: Option<String>,` (values `"local"`/`"parameter"`/`"global"`). Default to `None` at every existing construction site (locals/params in `scope.rs` get `Some("local")`/`Some("parameter")`; promoted globals in Task 3 get `Some("global")`).

- [ ] **Step 2: Mirror `scope` onto `L3RecordVariable`** (`l3_workspace.rs:124-158`) and forward it in the projection (`:840-852`).

- [ ] **Step 3: Add `is_temporary: bool` to `L3Table`** (`l3_workspace.rs:112-122`, default `false`) and to `AbiTable` (`symbol_reference.rs`, default `false`). Add `source_table_temporary: Option<bool>` to `L3Object` (`:36-79`).

- [ ] **Step 4: Add temp markers to ABI param.** In `symbol_reference.rs:41-47` add `pub is_temporary: bool` to `AbiParameter` (default `false`); add `#[serde(rename = "Temporary")] temporary: Option<bool>` to `RawTypeDef` (`:140-144`).

- [ ] **Step 5: Build.** Run: `cargo build`. Expected: compiles (all new fields defaulted at construction sites). Fix any missing-field errors by defaulting.

- [ ] **Step 6: Full suite.** Run: `cargo test`. Expected: GREEN (no behavior change yet; serialized output unchanged because all new fields are `None`/`false` + `skip_serializing_if` where applicable — `is_temporary: bool` on L3Table is internal, not serialized in goldens unless L3Table is golden-serialized; if it is, defer the moved golden to Task 19).

- [ ] **Step 7: rustfmt + commit.** `rustfmt` the 3 changed files. Commit: `feat(engine-ts1): additive model fields — recordVar scope, table is_temporary, page source_table_temporary, ABI param temporary`.

---

## Task 2: L2 capture — `extract_object_globals` reads `temporary_keyword`

**Files:**
- Modify: `src/engine/l2/scope.rs` (`extract_object_globals:242-277`)
- Test: `tests/temp_state_capture.rs` (Create)

- [ ] **Step 1: Write failing test.** Create `tests/temp_state_capture.rs`. Fixture: codeunit with global `Buf: Record "Bar" temporary;` and global `Phys: Record "Bar";`. Drive L2 features for the object (use the L2 projection entry the r1a tests use). Assert: the global record var `buf` has `temp_state.kind == "known" && value == Some(true)` and `phys` has `Known(false)`. NOTE: `extract_object_globals` currently returns `PVariableSymbol` (scalar), not `PRecordVariable`. Decide the capture shape: globals must surface as record vars with temp_state. The cleanest path is a NEW function `extract_object_global_record_vars(object_node, ...) -> Vec<PRecordVariable>` that mirrors the local record-var walk (`scope.rs:157-196`) but over object-level `var_section`s, OR extend `PVariableSymbol` with `temp_state`. CHOOSE the record-var route (Task 3 promotes `PRecordVariable`s). Assert against that new function's output.

- [ ] **Step 2: Run test, verify fail.** `cargo test --test temp_state_capture`. Expected: FAIL (function/field absent).

- [ ] **Step 3: Implement.** Add `extract_object_global_record_vars` in `scope.rs`: iterate object `named_children` for `var_section` (this already covers `protected`/`local` leading-child sections per RV-8 — no extra work), then each `variable_declaration` whose `type_specification` has a `record_type` child. Reuse the EXACT keyword-read pattern from `:184-186`: `named_children(record_type_node).iter().any(|c| c.kind() == "temporary_keyword")`. Build `PRecordVariable { id: <object-scoped id>, name, table_name, temp_state: ts_known(is_temporary), is_parameter: false, parameter_index: None, scope: Some("global".into()) }`. Use an object-scoped id distinct from routine-scoped ids (e.g. `format!("{}/grv/{}", object_id, name.to_lowercase())`) — Task 3 re-keys per routine via `encodeRecordVariableId(routineId, name)` equivalent. Document the conservative gaps (RV-8): `#if` object var sections (`preproc_conditional_var_block`) and dataitem-scoped var sections in Reports/Queries are NOT walked → fall to Unknown → fires. Add an inline comment listing them.

- [ ] **Step 4: Run test, verify pass.** `cargo test --test temp_state_capture`. Expected: PASS.

- [ ] **Step 5: Full suite.** `cargo test`. Expected: GREEN (new function not yet wired into projection — no behavior change).

- [ ] **Step 6: rustfmt + commit.** Commit: `feat(engine-ts2): capture temporary_keyword on object-global record vars (G1)`.

---

## Task 3: L3 promotion — promote object-global record vars into each routine

**Files:**
- Modify: `src/engine/l3/l3_workspace.rs` (call `extract_object_global_record_vars`, promote into each routine's `record_variables`)
- Test: extend `tests/temp_state_capture.rs`

- [ ] **Step 1: Write failing test.** Add a case: a procedure (declaring NO local named `Buf`) contains an op on the global `Buf` (temporary). Assert the routine's resolved `record_variables` contains a promoted entry for `buf` with `scope==Some("global")` and `Known(true)`, AND (after wiring) the op on `buf` resolves to `Known(true)`. Also assert shadowing: a local `Buf` (physical) in another procedure overrides the global → that routine's `buf` record var is `Known(false)`.

- [ ] **Step 2: Run test, verify fail.** Expected: FAIL (globals not promoted; op on member var falls to Unknown — this is the CDO bug class).

- [ ] **Step 3: Implement promotion.** In `l3_workspace.rs` where per-routine `record_variables` are assembled (`:840-852`): after collecting the routine's own record vars (params + locals), promote each object-global record var into the routine UNLESS a same-named local/param already exists. **Shadowing order (RV-5):** insert globals FIRST, then locals/params OVERWRITE (last-wins by insertion into a name-keyed map) — innermost wins. Re-key each promoted global to a per-routine id (`format!("{}/rv/{}", routine_id, name.to_lowercase())`) and set `scope: Some("global")`. The existing op-backfill (`l2/mod.rs:346-360` and the L3 forward) then resolves member-var ops with no new mechanism. Ensure the promoted record vars also carry `table_name` so `record_types.rs` resolves their `table_id`.

- [ ] **Step 4: Run test, verify pass.** Expected: PASS — member-var op now `Known(true)`; shadowed local stays `Known(false)`.

- [ ] **Step 5: Full suite.** `cargo test`. Expected: member-var ops flipping Unknown→Known will MOVE r1a/r2a/r4/digest goldens. If failures are confined to those golden suites AND are explainable Unknown→Known(true/false) flips on member vars, that is the DESIGNED change — note each for Task 19 and proceed (do NOT hand-edit goldens here). If any NON-temp-related golden moves, STOP and investigate (regression). Confirm the new unit test + `temp_state_shadowing` pass.

- [ ] **Step 6: rustfmt + commit.** Commit: `feat(engine-ts3): promote object-global record vars into routines, globals-first/locals-overwrite shadowing (G2)`.

---

## Task 4: native `TableType = Temporary` capture + table-level override precedence

**Files:**
- Modify: `src/engine/l3/l3_workspace.rs` (`index_table` reads `TableType` property → `L3Table.is_temporary`)
- Modify: `src/engine/l3/record_types.rs` (table-level override rule at op resolution — applies to params too, RV-8)
- Test: extend `tests/temp_state_capture.rs`

- [ ] **Step 1: Write failing test.** Fixture: a table object with property `TableType = Temporary;`. A codeunit var `Rec: Record "ThatTable";` (NO `temporary` keyword) with an op. Assert the op resolves `Known(true)` (table-level override beats the absent keyword). Second case: a by-var PARAM `var Rec: Record "ThatTable"` (no keyword) → must also resolve `Known(true)` at L3 (override supersedes the `PD(i)` stamped at L2 — RV-8 "param typed on a temp table").

- [ ] **Step 2: Run test, verify fail.** Expected: FAIL (TableType unread; param stays PD).

- [ ] **Step 3: Implement native capture.** In `index_table` (`l3_workspace.rs:487-596`) read the object-level `TableType` property (use the same property-read helper `classify_field`/`read_object_property` patterns; structural property node, value `"Temporary"` case-insensitive exact-match — NOT string-sniffing). Set `L3Table.is_temporary = true`.

- [ ] **Step 4: Implement override at op resolution.** In `record_types.rs`, AFTER an op's `table_id` is resolved (all passes), RE-RUN the override: if the op's resolved table has `is_temporary == true`, force `op.temp_state = Known(true)` (and the record var's, for consistency) REGARDLESS of the var modifier or a stamped `PD(i)`. This is the "one precedence rule everywhere" — table-level temp ⇒ Known(true). It must run where `table_id` is known (L3), superseding L2's `PD(i)` for params.

- [ ] **Step 5: Run test, verify pass.** Expected: PASS.

- [ ] **Step 6: Full suite.** `cargo test`. New table-temp golden moves → defer to Task 19. Unit tests green.

- [ ] **Step 7: rustfmt + commit.** Commit: `feat(engine-ts4): native TableType=Temporary capture + table-level override precedence incl. params (G3, RV-8)`.

---

## Task 5: page `SourceTableTemporary` → implicit Rec/xRec `Known(true)`

**Files:**
- Modify: `src/engine/l3/l3_workspace.rs` (read `SourceTableTemporary` page property → `L3Object.source_table_temporary`)
- Modify: `src/engine/l3/record_types.rs` (implicit Rec/xRec resolution `:131-173` → Known(true) when flag set)
- Test: extend `tests/temp_state_capture.rs`

- [ ] **Step 1: Write failing test.** Fixture: a Page object with `SourceTableTemporary = true;` and a trigger/procedure with implicit `Rec.<op>` and `xRec.<op>`. Assert BOTH `rec` and `xrec` ops resolve `Known(true)` (RV-8: xRec alongside Rec).

- [ ] **Step 2: Run test, verify fail.** Expected: FAIL.

- [ ] **Step 3: Implement.** Read `SourceTableTemporary` page property alongside `SourceTable` (`l3_workspace.rs:699-703`) → `L3Object.source_table_temporary = Some(true)`. In `record_types.rs` implicit-Rec pass (`:131-173`): when resolving `rec`/`xrec` for a Page whose `source_table_temporary == Some(true)`, set those ops' `temp_state = Known(true)` (in addition to `table_id`).

- [ ] **Step 4: Run test, verify pass.** Expected: PASS (both rec and xrec).

- [ ] **Step 5: Full suite.** `cargo test`. Page-temp golden moves → defer to Task 19.

- [ ] **Step 6: rustfmt + commit.** Commit: `feat(engine-ts5): page SourceTableTemporary → implicit Rec/xRec Known(true) (G4)`.

---

## Task 6: ABI capture — `TypeDefinition.Temporary` params + `TableType` tables + net-new ABI record-var modeling

**Files:**
- Modify: `src/engine/deps/symbol_reference.rs` (read `Temporary` on params/return types; read `TableType` property on tables)
- Modify: the ABI dependency projection (where `AbiRoutine` → L3 routine; currently `record_variables: []`, no per-param tempState — net-new modeling). Find the Rust equivalent of TS `dependency-projection.ts:101-117`.
- Test: `tests/temp_state_abi.rs` (Create) using a real or synthetic `.app` symbol fixture.

- [ ] **Step 1: Write failing test.** Use the symbol-reference test fixtures already in the repo (the r2.5b dep `.app`s, or a hand-authored `SymbolReference.json` snippet). Assert: (a) a table with `{"Name":"TableType","Value":"Temporary"}` → `AbiTable.is_temporary == true`; (b) a record param with `TypeDefinition: { Name: "Record", "Temporary": true }` → `AbiParameter.is_temporary == true`; (c) the projected cross-app routine exposes a record var / per-param tempState of `Known(true)` for that param (native+ABI shape parity). For a by-var record param WITHOUT `Temporary` → `PD(i)`; by-value → `Known(false)`.

- [ ] **Step 2: Run test, verify fail.** Expected: FAIL (markers unread; ABI routines have empty record vars).

- [ ] **Step 3: Implement ABI reads.** In `symbol_reference.rs`: `RawTypeDef` already gained `Temporary: Option<bool>` (Task 1) — populate `AbiParameter.is_temporary` from `param.type_definition.temporary`. Read the `TableType` property on raw table objects (`:720-754`) → `AbiTable.is_temporary` (property `{"Name":"TableType","Value":"Temporary"}`, exact match).

- [ ] **Step 4: Implement net-new ABI record-var modeling.** In the ABI→L3 projection, synthesize per-param `record_variables` for record-typed params with tempState per the native rule: keyword/`Temporary:true` → `Known(true)`; by-var no marker → `PD(index)`; by-value → `Known(false)`; table-level `is_temporary` override → `Known(true)`. This is the budgeted net-new modeling (RV-4) — mirror the native param walk's shape. Document if the symbol format lacks a needed field (fall to Unknown).

- [ ] **Step 5: Run test, verify pass.** Expected: PASS.

- [ ] **Step 6: Full suite.** `cargo test`. Cross-app r2.5b/r3a/r4 goldens may move → defer to Task 19.

- [ ] **Step 7: rustfmt + commit.** Commit: `feat(engine-ts6): ABI reads TypeDefinition.Temporary + TableType; net-new per-param record-var tempState modeling (G7, RV-4)`.

---

## Task 7: L4 PD substitution at effect composition (Component 2, RV-7)

**Files:**
- Modify: `src/engine/l4/summary_runner.rs` (`compose_routine`, edge-inheritance loop `:380-459`)
- Test: `tests/temp_state_substitution.rs` (Create)

The substitution table (RV-7), applied per-callsite to each inherited callee effect whose `temp_state == PD(i)`:

| binding of callee arg `i` (in the caller's frame) | substituted tempState |
|---|---|
| caller's known-temp var (incl. promoted member var, table-level override) | `Known(true)` |
| caller's known-physical var | `Known(false)` |
| caller's own by-var param `j` (keyword → resolve via caller recordVar) | `Known(true)` if caller param keyword/temp; else re-symbolize `PD(j)` |
| unbindable / no binding / event-dispatch / interface / object-run / dynamic edge | `Unknown` |

- [ ] **Step 1: Write failing tests.** Create `tests/temp_state_substitution.rs`. Cases: (a) **PD upgrade chain**: temp caller var passed to a by-var helper that does an op on its param → caller's inherited effect resolves `Known(true)`. (b) **mixed callers**: caller A (temp) + caller B (physical) call the same helper → TWO distinct inherited effects `(opId, Known(true))` and `(opId, Known(false))` (dedupe key is `(operationId, resolvedTempState)`). (c) **event-subscriber PD stays unresolved**: an event-dispatch edge carries no callsite_id → inherited effect `Unknown`. (d) **recursion through PD**: a PD chasing itself around an SCC stabilizes as PD/Unknown, never gains Known. (e) **by-value-of-temp**: callee `Insert()` on a by-value param is PHYSICAL — passing a temp arg by VALUE must NOT make it Known(true) (the existing `Known(false)` by-value rule is load-bearing). Drive these via the L4 summary entry the r3a tests use; assert on the composed routine summary's db_effects tempStates.

- [ ] **Step 2: Run tests, verify fail.** Expected: FAIL (PD inherited verbatim, foreign-frame index).

- [ ] **Step 3: Implement substitution.** In the edge-inheritance loop (`:380-459`): for each `edge`, recover its callsite via `edge.callsite_id` → the matching `routine.call_sites[*]` and its `argument_bindings`. For each inherited callee `db_effect` with `temp_state == ParameterDependent(i)`: look up the binding for callee param `i`, apply the table above. Use `binding.source_temp_state` (when the arg is a local/recVar source) and the Task-11 param-source resolution (when the arg is the caller's own param). For edge kinds with `callsite_id == None` (event-dispatch) or kinds `interface`/`codeunit-run`/`page-run`/`report-run`/`dynamic` → `Unknown`. Emit ONE inherited effect per distinct substitution result; dedupe by `(operation_id, resolved_temp_state)` (the effect key already includes temp_state via `key_fragment`). Beware the per-callsite flood guard — the dedup bound keeps per-op resolved-state space finite ({Known(t),Known(f),Unknown} ∪ PD(callerParams)).

- [ ] **Step 4: Run tests, verify pass.** Expected: PASS all five cases.

- [ ] **Step 5: Perf check.** Run the existing Base-App-scale perf test / the r3a SCC tests. Confirm no fixed-point blowup (the `MAX_FIXED_POINT_ITERATIONS` cap is not hit on real corpora). If a perf golden exists, compare before/after.

- [ ] **Step 6: Full suite.** `cargo test`. R3a trace-oracle + digest goldens move for PD-touching SCCs → defer to Task 19.

- [ ] **Step 7: rustfmt + commit.** Commit: `feat(engine-ts7): substitute ParameterDependent per-callsite at effect inheritance, dedupe by (op,tempState) (G5, RV-7)`.

---

## Task 8: param-source binding resolution (RV-7 binding gap)

**Files:**
- Modify: the binding builder (`PCallArgumentBinding` construction — find the Rust equivalent of TS `intraprocedural-body.ts:188,220,238`) so `source_kind` honors `scope` (RV-8 diagnostic) AND param-source args resolve through the caller's own recordVar tempState.
- Modify: `src/engine/l4/summary_runner.rs` substitution to consume it.
- Test: extend `tests/temp_state_substitution.rs`

- [ ] **Step 1: Write failing test.** Case: caller has a `var Rec: Record X temporary` PARAM (keyword) that it forwards as an arg to a by-var helper which ops on it. The forwarded arg is the caller's OWN param (carries `source_parameter_index`, no `source_temp_state`). Assert the inherited effect resolves `Known(true)` (forwarding case), NOT Unknown.

- [ ] **Step 2: Run test, verify fail.** Expected: FAIL (param-source arg has no temp_state → falls to PD/Unknown, defeating forwarding).

- [ ] **Step 3: Implement.** At composition, when a binding has `source_parameter_index = Some(j)` and `source_temp_state == None`, resolve through the caller's own `record_variables[param j].temp_state`: keyword param → `Known(true)`; keyword-less by-var param → re-symbolize `PD(j)` (chains upward); by-value → `Known(false)`. Also fix the binding builder's hardcoded `source_kind: "local"` to honor the resolved var's `scope` (diagnostic-only mislabel otherwise, RV-8).

- [ ] **Step 4: Run test, verify pass.** Expected: PASS.

- [ ] **Step 5: Full suite.** `cargo test`. Defer moved goldens to Task 19.

- [ ] **Step 6: rustfmt + commit.** Commit: `feat(engine-ts8): resolve param-source arg bindings through caller recordVars; scope-honest sourceKind (RV-7 binding gap)`.

---

## Task 9: L5 `resolve_temp_along_path` + walker exposes callee-param index per hop (Component 3, RV-6)

**Files:**
- Modify: `src/engine/l5/path_walker.rs` (`WalkResult`/`EvidenceStep` expose callee-param index per hop)
- Create/Modify: a shared helper `resolve_temp_along_path(path, terminal_op) -> TempStateKind` in the path-walker module
- Test: `tests/temp_state_path.rs` (Create)

- [ ] **Step 1: Write failing test.** Case (mixed callers, per-path truth): helper does a db-op on a by-var param; caller A passes a temp var, caller B passes a physical var. Walk both evidence paths. Assert `resolve_temp_along_path(pathA, op) == Known(true)` and `resolve_temp_along_path(pathB, op) == Known(false)`. Case (root PD): a path whose root is an entry parameter (no caller) stays PD at root → resolves `Unknown`.

- [ ] **Step 2: Run test, verify fail.** Expected: FAIL (helper does not exist).

- [ ] **Step 3: Implement walker output.** Each hop's `EvidenceStep` already has `callsite_id`. Expose, per hop, the callee-param index needed to step frames (net-new walker output per RV-6) — derive from the callsite's argument bindings keyed by `callsite_id`. Add the field to `EvidenceStep` or a parallel per-hop structure on `WalkResult`.

- [ ] **Step 4: Implement the helper.** `resolve_temp_along_path(path, terminal_op)`: start from `terminal_op.temp_state`; if `PD(i)`, step ONE frame toward the path root via that hop's `callsite_id` + the callsite's argument binding for param `i` (SAME substitution table as Task 7); repeat until `Known`/`Unknown` or the path root. Still-PD at root (entry parameter, caller unknown) → `Unknown`.

- [ ] **Step 5: Run test, verify pass.** Expected: PASS both cases.

- [ ] **Step 6: Full suite.** `cargo test`. The helper is not yet consumed by detectors → no golden move expected; if walker output serialization changes, defer to Task 19.

- [ ] **Step 7: rustfmt + commit.** Commit: `feat(engine-ts9): shared resolve_temp_along_path L5 helper + per-hop callee-param index (Component 3, RV-6)`.

---

## Task 10: d1 consumes path-resolved temp state + worst-severity merge-tie + note wording

**Files:**
- Modify: `src/engine/l5/detectors/d1.rs` (read path-resolved state; merge-tie rule)
- Test: extend `tests/temp_state_path.rs`

- [ ] **Step 1: Write failing test.** Mixed-callers scenario surfaced as d1 findings: the caller-A path reports info (temp), the caller-B path fires at normal severity (physical). After `merge_by_terminal` collapses to one finding, assert: WORST severity wins (normal/warning, not info) AND the note lists both verdicts (e.g. "temporary via CallerA path; physical via CallerB path").

- [ ] **Step 2: Run test, verify fail.** Expected: FAIL (d1 reads `terminalOp.tempState` directly, no path resolution; merge may pick wrong severity).

- [ ] **Step 3: Implement.** d1's temp policy (`d1.rs:172-207`) consumes `resolve_temp_along_path(path, terminal_op)` instead of the raw `op.temp_state`. In the `merge_by_terminal` consumption (`path_merge.rs`/`d1.rs:724`): when merged paths DISAGREE on temp-derived severity, the WORST severity wins (deterministic), and the temp note lists both verdicts. Keep the honest note wording ("(temporary record — not a SQL round-trip)" / "(temp state uncertain)"). NO detector POLICY semantics change beyond per-path resolution + merge-tie.

- [ ] **Step 4: Run test, verify pass.** Expected: PASS.

- [ ] **Step 5: Full suite.** `cargo test`. d1 r4 goldens move for any multi-caller PD finding → defer to Task 19.

- [ ] **Step 6: rustfmt + commit.** Commit: `feat(engine-ts10): d1 reads path-resolved tempState; worst-severity merge-tie + dual-verdict note (RV-6)`.

---

## Task 11: RV-1 CalcFields/FlowField gate in d1

**Files:**
- Modify: `src/engine/l5/detectors/d1.rs` (gate the temp downgrade for CalcFields/SetAutoCalcFields)
- Test: `tests/temp_state_calcfields.rs` (Create)

- [ ] **Step 1: Write failing tests.** (a) **CalcFields-Blob-on-temp**: temp record, `CalcFields("File Blob")` where `"File Blob"` is a Blob field (`field_class == "Normal"`, `is_blob_like`) → MUST downgrade to info (in-memory). (b) **CalcFields-FlowField-on-temp**: temp record, `CalcFields("Amount")` where `"Amount"` is a `FlowField` → MUST keep firing at normal severity with the note "temporary record, but FlowField calculation queries the flow targets". (c) Unresolvable field arg on a temp CalcFields → keep firing (conservative).

- [ ] **Step 2: Run tests, verify fail.** Expected: FAIL (current d1 blanket-downgrades all temp ops including FlowField CalcFields).

- [ ] **Step 3: Implement gate.** In d1: for an op `CalcFields`/`SetAutoCalcFields` on a (path-resolved) `Known(true)` temp record, downgrade to info ONLY when EVERY named field argument resolves (via the table model: `op.table_id` → table fields → `field_class`) to `field_class != "FlowField"`. ANY FlowField field arg OR any unresolvable field arg ⇒ keep firing at normal severity with the FlowField note. Field args come from `op.field_arguments`/`field_argument_infos`; resolve names against the resolved table's fields (mirror d22's field-by-name lookup). field_class is modeled on both native (`classify_field`) and ABI (`parse_field`) sides — works for cross-app tables too.

- [ ] **Step 4: Run tests, verify pass.** Expected: PASS all three.

- [ ] **Step 5: Full suite.** `cargo test`. Defer moved goldens to Task 19.

- [ ] **Step 6: rustfmt + commit.** Commit: `feat(engine-ts11): RV-1 CalcFields/FlowField gate — temp downgrade only when no FlowField field arg`.

---

## Task 12: RecordRef GetTable / OpenTemporary (Component 4, local-only)

**Files:**
- Modify: RecordRef modeling site (find where `RecRef.Open`/`GetTable` ops are modeled — likely `l3_workspace.rs` body walk or a RecordRef-specific resolver)
- Test: `tests/temp_state_recordref.rs` (Create)

- [ ] **Step 1: Write failing test.** (a) `RecRef.GetTable(TempRec)` then `RecRef.<op>` in the SAME routine, unconditional flow → the RecRef op inherits `TempRec`'s tempState (`Known(true)`). (b) `RecRef.Open(no, true)` (OpenTemporary form) → `Known(true)`; plain `RecRef.Open(no)` → `Known(false)`. (c) Anything beyond local unconditional flow → `Unknown`.

- [ ] **Step 2: Run test, verify fail.** Expected: FAIL.

- [ ] **Step 3: Implement.** Locally-determinable only (same routine, unconditional flow): `GetTable(SomeRec)` → subsequent RecRef ops inherit `SomeRec`'s tempState; `Open(no, true)` → `Known(true)`; plain `Open` → `Known(false)`. Anything else → `Unknown`. Out of scope: `Copy(..., ShareTable)` aliasing (declaration-bound tempness; documented non-goal).

- [ ] **Step 4: Run test, verify pass.** Expected: PASS.

- [ ] **Step 5: Full suite.** `cargo test`. Defer moved goldens to Task 19.

- [ ] **Step 6: rustfmt + commit.** Commit: `feat(engine-ts12): RecordRef GetTable/OpenTemporary local-only tempState (G6)`.

---

## Task 13: fixture corpus

**Files:**
- Create: fixtures under `tests/fixtures/temp-state/` (or the corpus pattern the differential tests use), one per listed case.

- [ ] **Step 1: Author fixtures.** One workspace per case (RV-10 locked list): member-temp + member-non-temp; `TableType=Temporary` table; `SourceTableTemporary` page (Rec + xRec); PD upgrade chain (temp caller → by-var helper); **mixed callers** (temp + physical → two effects, two path verdicts); recursion-through-PD; event-subscriber-PD stays unresolved; RecordRef `GetTable`; **local-shadows-global**; **by-value-of-temp** (callee `Insert()` on by-value param is PHYSICAL — must NOT suppress); **CalcFields-FlowField-on-temp** (must keep firing); **CalcFields-Blob-on-temp** (must downgrade). Reuse the fixtures already created in Tasks 0-12 where possible; this task fills the gaps and consolidates.

- [ ] **Step 2: Write per-fixture assertions** as integration tests (the per-layer unit tests already cover capture/promotion/shadowing/substitution/path-resolution; this task adds the end-to-end `analyze`-level assertions where useful).

- [ ] **Step 3: Run.** `cargo test`. Expected: GREEN.

- [ ] **Step 4: rustfmt + commit.** Commit: `test(engine-ts13): temp-state fixture corpus (member/table/page/PD/mixed/recursion/event/recordref/shadow/by-value/calcfields)`.

---

## Task 14: metamorphic soundness oracle (RV-2 carve-out)

**Files:**
- Create: `tests/temp_state_oracle.rs`

- [ ] **Step 1: Write the oracle test.** For each temp-state fixture, mechanically produce an edited copy with `temporary` ADDED to a record declaration (table/var). Run `analyze` on both. Assert the RV-2 property: adding `temporary` may only **remove or downgrade** findings, EXCEPT findings on `CalcFields`/`SetAutoCalcFields` ops whose field arguments include a FlowField — those must be **INVARIANT** under the edit. Assert BOTH directions: (a) the suppressible class shrinks-or-equal; (b) the FlowField-CalcFields class is unchanged. Use the analyze entry (`run_analyze`) the cli tests use; compare finding sets by `(detector, location, severity)`.

- [ ] **Step 2: Run.** `cargo test --test temp_state_oracle`. Expected: PASS (if it fails, a suppression-direction violation exists — STOP and fix the offending task, do NOT weaken the oracle).

- [ ] **Step 3: Full suite + rustfmt + commit.** Commit: `test(engine-ts14): metamorphic soundness oracle with RV-2 FlowField carve-out`.

---

## Task 15: CDO real-world acceptance

**Files:**
- Create: an acceptance test or documented manual procedure. (The CDO workspace is external: `U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud`.)

- [ ] **Step 1: Build release + run.** `cargo build --release`. Run: `target/release/alsem.exe analyze U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud --format terminal`.

- [ ] **Step 2: Assert acceptance criteria (manual/scripted):**
  - `CDOCDNEDocEmbedReqSubs.ClearFiles` d33 **critical is GONE** (member temp var `Files` now Known(true) via Task 3 capture — d33's existing suppression fires).
  - `LoadFiles`' CalcFields-on-Blob (`Files.CalcFields("File Blob", …)`) **drops to info** (Task 11 gate: Blob field, not FlowField → downgrade).
  - A synthetic CalcFields-on-FlowField-on-temp case **still fires** (Task 11 / Task 14 cover this; if not present in CDO, the synthetic fixture from Task 13 suffices).

- [ ] **Step 3: Record the before/after** in `CHANGELOG.md` and/or a short note in `docs/`. Commit any test/doc artifact: `test(engine-ts15): CDO acceptance — ClearFiles critical gone, LoadFiles Blob→info, FlowField-on-temp still fires`.

---

## Task 16: golden rebaseline via env-gated regen path + cache version bump

**Files:**
- Create: an env-gated regen path (mirror `tests/r2_5b_refresh.rs` `#[ignore]` + `AL_SEM_DIR` pattern, but Rust-OWNED: write the CURRENT engine output as the golden, not copy from al-sem).
- Modify: `src/engine/gate/cache_prune.rs:46` (`"17"` → `"18"`); `tests/cli_c_cache_differential.rs` (`:354` and the stale-replace `:502,532`).
- Modify: the moved golden files under `tests/*-goldens/` (regenerated, then REVIEWED).

- [ ] **Step 1: Build the regen path.** Add an env-gated (`REGEN_TEMP_GOLDENS=1` or similar, `#[ignore]`-decorated like `r2_5b_refresh`) mode that, instead of asserting, writes the actual current engine output to the affected golden files (r0/r1a/r2a/r2b/r2c/r2d/r2.5b/r3a*/r4/cli-* + digest tempState merge + R3a-2 trace-oracle baselines). Mirror the existing differential harness serialization exactly so byte-format matches.

- [ ] **Step 2: Bump cache version.** `CACHE_VERSION_SYMBOL_READER` `"17"` → `"18"` in `cache_prune.rs:46`; update `cli_c_cache_differential.rs` `current_versions_json()` (`:354`) and the stale-replace literals (`:502,532`) accordingly.

- [ ] **Step 3: Regenerate.** Run the regen path. It rewrites the moved goldens.

- [ ] **Step 4: REVIEW the diff finding-by-finding (the review IS the verification).** `git diff tests/*-goldens/`. EVERY moved golden must be explainable by a designed semantic change: member-var ops Unknown→Known(true/false); table/page temp flips; PD substitution results; per-path d1 severities; the CalcFields/FlowField gate. If ANY moved golden is NOT explainable by a designed change → STOP, it is a regression — fix the offending task. The digest tempState "physical-if-ANY" asymmetry (`digest.rs:1911-2037`) MUST be preserved (it already encodes the right suppression direction).

- [ ] **Step 5: Full suite.** `cargo test`. Expected: GREEN (goldens now match the new engine output).

- [ ] **Step 6: rustfmt + commit.** Stage the regen path + cache-version files + the reviewed goldens. Commit: `chore(engine-ts16): rebaseline goldens for temp-state epoch + bump symbolReader cache 17→18`. Include in the message a summary of the golden-diff review (which suites moved and why).

---

## Task 17: final integration, release rebuild, changelog

- [ ] **Step 1: Full clean test.** `cargo test`. Expected: GREEN.
- [ ] **Step 2: Clippy + fmt check.** `cargo clippy --all-targets --all-features` (no new warnings on changed code); confirm all changed files were `rustfmt`-ed.
- [ ] **Step 3: Release rebuild** (al-perf depends on `target/release/alsem.exe`). `cargo build --release`.
- [ ] **Step 4: CHANGELOG.md** — ensure a complete entry for the epoch under Added/Changed/Fixed (member/global/table/page/ABI temp capture; PD substitution; path-time resolution; CalcFields/FlowField gate; RecordRef; first-wins shadowing fix; cache bump).
- [ ] **Step 5: Commit** any remaining: `docs(engine-ts17): changelog for temp-state tracking epoch`.
- [ ] **Step 6: Fast-forward master** per the worktree mechanic: `git -C U:/Git/al-call-hierarchy merge --ff-only engine`. (Do NOT push — user-only.)

---

## Self-review (planner checklist — done)

**Spec coverage:** RV-1 (Task 11), RV-2 (Task 14), RV-3 (documented contract-trust — keep Known(true) for keyword param, inline doc in Task 6/8), RV-4 (Task 6), RV-5 (Task 0), RV-6 (Tasks 9-10), RV-7 (Tasks 7-8), RV-8 (Tasks 2,3,4,5,8 — scope field, gaps documented, param-on-temp-table override, xRec), RV-9 (Task 16 breadth), RV-10 (Tasks 13 fixtures, 10 tempered claims). Components 1-4 + detector policy + oracle + baselines all mapped. Implementation order (0)-(8) preserved.

**Placeholder scan:** test code is described with concrete fixtures + assertions; impl steps cite exact file:line + the spec's substitution table inline. TDD contract (tests first) is the spec for impl code the implementer writes against the real source.

**Type consistency:** `temp_state`/`PTempState`/`TempStateKind`/`Known`/`PD(i)`/`Unknown`, `scope: Option<String>`, `is_temporary: bool`, `source_table_temporary: Option<bool>`, `resolve_temp_along_path`, `(operation_id, resolved_temp_state)` dedup key — used consistently across tasks.

**Known risk:** several impl steps cannot ship fully-literal Rust without the implementer reading current source (port of this size). Mitigation: TDD-first — each task's tests are the executable contract; implementers read the cited file:line and write impl to pass. This matches subagent-driven-development.
