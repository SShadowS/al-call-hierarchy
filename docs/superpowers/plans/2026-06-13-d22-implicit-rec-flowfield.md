# d22 implicit-Rec FlowField FN Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `Rec.<FlowField>` reads on the implicit trigger/page `Rec` visible to d22 (FlowField-without-CalcFields), fixing the silent false negative recorded in `docs/detector-audit.md` (d22 FN).

**Architecture:** Register the implicit `Rec` of a table-trigger / page (SourceTable) / page-extension routine as a real `RecordVariable` (so `Rec.Field` is captured as a `PFieldAccess` and its table resolves at L3). Gate d3 (missing-SetLoadFields) so the new implicit-Rec accesses don't produce FPs (the platform already loads `Rec` in those triggers). Rebaseline the affected L2 / R3a / r4 goldens.

**Tech Stack:** Rust, tree-sitter-al V2, REGEN_TEMP_GOLDENS env-gated rebaseline.

**Branch:** `engine-d22` (off `engine`).

---

## Design decision (chosen by user)

FULL design: register an implicit `Rec` record variable (vs. the narrower per-PFieldAccess table_id). More complete; requires a d3 trigger-Rec gate + R3a rebaseline.

## Key facts (verified during investigation)

- `implicit_base_receiver` (`src/engine/l2/mod.rs:127`) already returns a `"Rec"` frame for: table/tableext **trigger**, **page** with SourceTable, **pageextension**.
- `record_var_names` (`mod.rs:211`) is built from `extract_record_variables` — does NOT include the implicit Rec → `Rec.Field` is dropped at `body_walk.rs:894-899`.
- `RecordVariable` (`scope.rs`) carries `table_name: Option<String>`, resolved to `table_id` at L3 (same path declared record vars use).
- Implicit Rec's table name: **table trigger → the object's own name** (`extract_object_name`, available at `l2_workspace.rs:472`); **page / pageext → `source_table_name`** (already threaded to `project_routine_features`).
- d22 (`d22.rs:70`) resolves the table via `routine.record_variables["rec"].table_id` → works once Rec is a record var.
- d11 / d37 already gate trigger-Rec via `is_platform_loaded_trigger_rec` (`detectors/mod.rs`). d3 does NOT — it would start flagging implicit-Rec reads (FP) → needs a gate.
- `apply_field_read` (cfg_walker) keys on param name; "Rec" is not a param → L4 summaries unaffected by the new accesses.

---

## Task 1: L2 — register the implicit Rec record variable

**Files:**
- Modify: `src/engine/l2/scope.rs` (`extract_record_variables` — add implicit-Rec push) OR `src/engine/l2/mod.rs` (`project_routine_features` — push after the call; preferred, keeps `extract_record_variables` pure).
- Modify: `src/engine/l2/mod.rs:178` (`project_routine_features` signature — add `object_name: &str`).
- Modify: `src/engine/l2/l2_workspace.rs:543` (pass `name`).
- Test: `tests/gap_audit_d22_implicit_rec.rs` (new).

- [ ] **Step 1: Failing test** — a table-trigger `OnAfterGetRecord` doing `Rec."Balance (LCY)"` (FlowField) produces a d22 finding. (Will fail: no field access captured.) Also assert a declared-var control still works.
- [ ] **Step 2:** Add `object_name` param to `project_routine_features`; thread `name` from `l2_workspace.rs`.
- [ ] **Step 3:** In `project_routine_features`, after `extract_record_variables`, compute the implicit-Rec table name (table-trigger → `object_name`; page/pageext → `source_table_name`) and, when `implicit_base_receiver(...).is_some()` AND a table name is available AND no declared `Rec`/`rec` var already exists, push `RecordVariable{ id: "{routine_id}/rv/rec", name:"Rec", table_name: Some(tbl), temp_state: ts_known(false), is_parameter:false, parameter_index:None }`. Rebuild `record_var_names` to include it.
- [ ] **Step 4:** Run the L2 portion — confirm `Rec.Field` now appears in `field_accesses` and `record_variables` has Rec with the table name.
- [ ] **Step 5:** Commit.

**Edge cases:** pageext with no resolvable SourceTable at L2 → no table name → skip (Rec unresolved, d22 skips — conservative). A routine with an explicit `Rec` param/local (rare) → do not double-register.

## Task 2: d3 trigger-Rec gate (prevent new FPs)

**Files:**
- Modify: `src/engine/l5/detectors/d3.rs`.
- Test: extend `tests/gap_audit_d22_implicit_rec.rs` (d3 must NOT fire on implicit-Rec field read in a platform-loaded trigger).

- [ ] **Step 1: Failing test** — d3 fires on `Rec.Field` read in `OnAfterGetRecord` (the new FP). 
- [ ] **Step 2:** Add `is_platform_loaded_trigger_rec(routine, &fa.record_variable_name)` skip in d3's field-access loop (mirror d37/d11). 
- [ ] **Step 3:** Confirm test now passes; declared-var d3 control still fires.
- [ ] **Step 4:** Commit.

**Verify:** d11 (modify-without-get) on the implicit Rec — confirm its existing trigger-Rec gate covers the new record var (it should; `is_platform_loaded_trigger_rec` keys on routine kind + receiver). Add a control if not.

## Task 3: d22 — confirm the FN is fixed

**Files:** `tests/gap_audit_d22_implicit_rec.rs`.

- [ ] **Step 1:** d22 fires on `Rec.<FlowField>` in `OnAfterGetRecord` with no prior CalcFields.
- [ ] **Step 2:** Control — `CurrPage.… ` / `Rec.CalcFields("Balance (LCY)"); … Rec."Balance (LCY)"` suppressed.
- [ ] **Step 3:** Control — a NORMAL field read on implicit Rec does NOT fire d22.
- [ ] **Step 4:** Commit (tests only if not already committed).

## Task 4: Golden rebaseline

- [ ] **Step 1:** `cargo test` — enumerate every moved golden. Expect: L2 feature goldens (added implicit-Rec field accesses + the Rec record var) for trigger/page fixtures; R3a record-type projection (new record var/op); possibly r4 (d22/d3 deltas).
- [ ] **Step 2:** For EACH moved suite, inspect the diff — confirm changes are ONLY (a) added implicit-Rec field accesses, (b) the new Rec record var, (c) intended d22 additions / d3 unchanged. NO unintended detector movement (d1/d10/d33/etc.).
- [ ] **Step 3:** Regenerate via the suite's REGEN path (`REGEN_TEMP_GOLDENS=1` for r4; find the equivalent for L2/R3a). Vendored in-repo, never into al-sem.
- [ ] **Step 4:** `cargo test` fully green. Commit goldens separately with a clear rebaseline message.

## Task 5: Validate on real code

- [ ] **Step 1:** `cargo build --release`.
- [ ] **Step 2:** Re-run CDO analyze; expect d22 primary count to INCREASE (the FN was CDO-relevant). Confirm no unexpected detector regressions.
- [ ] **Step 3:** Update `docs/detector-audit.md` (mark d22 FIXED). Commit.

## Self-review checklist

- L2 PartialEq / serde: the new Rec record var + field accesses DO change serialized L2 — rebaseline required (not a serde-skip field). Confirm no OTHER serialized shape changed.
- Suppression-direction: d3 gate is the only new suppression — exact (platform-loaded trigger Rec). d22 gains are FN fixes (fire more), controls prove non-vacuous.
- Do not register Rec when its table can't resolve (keeps d22 conservative; no spurious unresolved-table noise beyond the existing skip stat).
