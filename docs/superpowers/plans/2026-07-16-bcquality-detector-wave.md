# BCQuality Detector Wave (d52–d64) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement all 13 detector candidates from `docs/2026-07-16-scanner-validation-and-bcquality-candidates.md` §4 (derived from microsoft/BCQuality, MIT) as L5 detectors d52–d64, each with its own fixture workspace, byte-matched Rust-owned golden, and registry/preset wiring, plus a new `bcquality` preset.

**Architecture:** Each detector is a new module under `src/engine/l5/detectors/` following the established shape (`detect_dNN(resolved: &L3Resolved, ctx: &DetectorContext) -> Result<DetectorOutput, DetectorError>`), registered in `registered_detectors()` (`detectors/mod.rs`) and in `src/engine/gate/presets.rs` name lists. Three small additive substrate forwards land first (Tasks 1–3) because later detectors consume them. Every detector gets one fixture workspace under `tests/r0-corpus/ws-dNN/` containing BOTH flagged and deliberately-unflagged cases, one golden under `tests/r4-goldens/ws-dNN.r4.golden.json` (regenerated via `REGEN_TEMP_GOLDENS=1`, inspected before commit), one `Smoke` entry in `tests/r4/r4_differential.rs`, and one neutral-fixture negative assertion.

**Tech Stack:** Rust (existing crate `al_call_hierarchy`), the L3/L4/L5 engine substrate (no tree-sitter access — detectors read the L3 model + `DetectorContext` only), cargo test / cargo nextest.

## Global Constraints

- Format touched files with `rustfmt <file>` — NEVER `cargo fmt` (CLAUDE.md).
- `cargo clippy --all-targets --all-features` must stay clean after every task (established permanent bar).
- **CHANGELOG.md must be updated** in every task that adds a feature (Keep a Changelog, group under `Added`).
- Stage files in logical groups; never `git add -A`. Never push/merge to `master` unrequested.
- Goldens are Rust-owned: `REGEN_TEMP_GOLDENS=1 cargo test --test r4` regenerates; **inspect the diff, never blind-bless**. `REGEN_TEMP_GOLDENS` is value-tested (`=1` exactly).
- Detector output must stay deterministic: findings sorted by `a.id.cmp(&b.id)` before return; every finding gets `fingerprint = Some(fp_index.fingerprint_of(&finding))`.
- Detector registry order is load-bearing for `detectorStats`: new DEFAULT detectors append AFTER the d45 entry (end of the default block) in `registered_detectors()`; new OPT-IN detectors append AFTER the d51 entry. `presets.rs` `DEFAULT_DETECTOR_NAMES` / `OPT_IN_DETECTOR_NAMES` must list the same names.
- Suppression direction: every uncertainty keeps the detector FIRING for bug-class detectors, but for these new advisory detectors every uncertainty must SKIP (fail-quiet) — they are precision-first BCQuality ports; note per-detector skips in `DetectorStats`.
- Severity strings in use: `"critical" | "high" | "medium" | "low" | "info"`. Confidence via `to_confidence(&[], "likely")` / `to_confidence(&[], "possible")`.
- New fixture app.json GUID convention: `11111111-0000-0000-0000-00000000d5NN0` style unique ids (existing fixtures use this pattern, e.g. ws-d33 = `...00000000d330`).

## Substrate reference (verified 2026-07-16, commit 6b1d890)

Facts every task relies on — verified by direct read, cite these instead of re-deriving:

- Detector template: `src/engine/l5/detectors/d33.rs` (record-op detector) and `d35.rs` (routine-kind detector). `Finding` fields: `id, root_cause_key, detector, title, root_cause, severity, confidence, primary_location, evidence_path, additional_paths: None, affected_objects, affected_tables, fix_options, provenance, actionable_anchor: None, fingerprint: None, event_kind: None, cross_extension_subscribers: None`.
- `DetectorStats::new(DETECTOR, candidates_considered, emitted)` + `stats.add_skip("name", count_u64)`; return `Ok(DetectorOutput::no_diag(findings, stats))`.
- Shared helpers in `detectors/mod.rs`: `anchor_of(&PAnchor, &L3Routine) -> SourceAnchor`, `before_anchor(a, b)`, `is_known_temp(op)`, `RECORD_FILTER_OPS`, `record_filtered_by_call_before(routine, ctx, var_name, anchor)`, `group_and_cap(findings, key_of, max)`.
- Attributes: `crate::engine::l3::al_attributes::{has_attribute, find_attribute}`; `L3Routine.attributes_parsed: Vec<AttributeInfo>`; `L3Routine.kind ∈ {"procedure","trigger","event-publisher","event-subscriber"}`.
- Loops: `PLoop { id, loop_type, source_anchor }` on `routine.loops`; `L3RecordOperation.loop_stack: Vec<String>` and `PCallSite.loop_stack: Vec<String>` (innermost = `.last()`); d34 is the loop-containment template.
- Temp proofs: `is_known_temp(op)` (`temp_state Known(true)`, includes the G-2 entry-guard upgrade applied by `record_types.rs`), `L3Routine.entry_temp_guard_receiver: Option<String>` (lowercased receiver of `if not X.IsTemporary() then Error(...)` entry guard), `ctx.closed_world_temp_params: HashSet<(String /*routine id*/, u32 /*param index*/)>` (G-19).
- Resolution join: `ctx.resolved_call_edge_by_callsite: HashMap<String, CallEdge>` (`edge.to: Option<String>` → `ctx.routine_by_id`). EMPTY in the cross-app context — detectors reading it are inert there (document per detector).
- Graph: `ctx.graph.edges_by_from: HashMap<String, Vec<CombinedEdge>>`; `CombinedEdge { from, to, kind, callsite_id, operation_id, event_id, subscriber_app_id, resolution }`; kinds: `direct | method | codeunit-run | report-run | page-run | interface | implicit-trigger | event-dispatch | dynamic`.
- Events: `ctx.event_graph.events: Vec<EventSymbol { id, publisher_routine_id: Option<String>, event_kind, parameters: Vec<L3Parameter>, .. }>`, `ctx.event_graph.edges: Vec<EventEdge { event_id, subscriber_routine_id, .. }>`.
- Params: `L3Routine.parameters: Vec<L3Parameter { index, name, type_text, is_var, is_record, table_name }>`.
- Variables: `L3Routine.variables: Vec<L3Variable { name, declared_type, is_parameter, parameter_index, initializer }>` covers params → locals → globals (globals ARE included; scope marker added by Task 3). `L3Routine.record_variables: Vec<L3RecordVariable>`.
- Assignments: `PVarAssignment { lhs_name /*lowercased*/, rhs_literal_value, source_anchor, rhs_identifier /*lowercased, whole-variable copies only, serde(skip)*/ }` on `routine.var_assignments`.
- Condition guards: `routine.condition_references: Vec<PConditionReference { identifier, condition_kind, statement_anchor, referenceAnchor }>` (d43's IsHandled-guard source).
- Call sites: `PCallSite { id, callee: PCallee::{Bare{name}|Member{receiver,method}}, argument_texts, argument_infos, argument_bindings, loop_stack, source_anchor, result_consumed (object-run only today), under_asserterror, .. }`. Statement-position marker added by Task 1.
- Object metadata: `L3Object { id, object_type, object_subtype /*Codeunit: "Install"/"Upgrade"/"Test"*/, page_type /*Page: "API"/...*/, source_table_temporary, .. }`. Native assembly harvests properties in `src/engine/l3/l3_workspace.rs` (~line 560–660) via the `ir_prop(name_lc)` closure over `o.properties` (property names LOWERCASED, e.g. `"pagetype"`, `"sourcetable"`); L3Object constructions: `l3_workspace.rs:645` (native), `src/engine/deps/cross_app_l3.rs:84` (dep), plus test constructors in `return_summary.rs`, `l5/fingerprint.rs`, `l3/receiver_type.rs` (×2), `l5/detectors/d17.rs`, `l5/detectors/d50.rs` — adding L3Object fields requires updating ALL of these.
- Test harness: `tests/r4/r4_differential.rs` — `Smoke { fixture, wave, detectors, ported, corpus_dir }` arrays iterated in `run_smoke_entry` loops; positives push into `ported_results` (anti-degenerate: byte-match AND ≥1 finding); 0-count goldens are run WITHOUT pushing (exempt). Fixtures live in `tests/r0-corpus/<fixture>/{app.json, src/*.al}`; goldens in `tests/r4-goldens/<fixture>.r4.golden.json`. There is also a `NEGATIVES` array (neutral-fixture, 0-finding assertions) near the wave arrays — follow its existing entry shape.
- Run one umbrella member: `cargo test --test r4 r4_differential::` (libtest module filter).

## File Structure (net-new / modified)

```
src/engine/l2/features.rs                 # Task 1: PCallSite.in_statement_position (+ manual PartialEq)
src/engine/l2/ir_walk.rs                  # Task 1: set the flag at PCallSite construction
src/engine/l3/l3_workspace.rs             # Task 2: L3Object.{single_instance, editable, insert_allowed,
                                          #   modify_allowed, delete_allowed, source_anchor}; Task 3: L3Variable.scope
src/engine/deps/cross_app_l3.rs           # Tasks 2–3: dep-side defaults (None)
src/engine/l5/detectors/mod.rs            # all tasks: pub mod dNN + registry entries + shared helper promotion
src/engine/l5/detectors/d52.rs .. d64.rs  # Tasks 4–16: one module per detector
src/engine/gate/presets.rs                # per-task name lists; Task 17: bcquality preset
tests/r0-corpus/ws-d52 .. ws-d64/         # per-detector fixtures
tests/r4-goldens/ws-d52.r4.golden.json .. # per-detector goldens (regen'd, inspected)
tests/r4/r4_differential.rs               # WAVE_BCQ arrays + loops + NEGATIVES entries
CHANGELOG.md                              # per-task Added entries
```

Execution order: Tasks 1–3 (substrate) → Tasks 4–16 in any order EXCEPT: Task 5 (d53) needs Task 1; Task 9 (d57) needs Tasks 2+3; Task 16 (d64) needs Task 2. Task 17–18 last.

---

### Task 1: Statement-position marker on call sites (substrate for d53)

`in_stmt_position` already exists in the ir_walk walker state (drives object-run `result_consumed`), but ordinary call sites don't record it. Add an internal-only field so d53 can tell `TryPost();` (result ignored) from `if TryPost() then` (consumed).

**Files:**
- Modify: `src/engine/l2/features.rs` (PCallSite, ~line 131)
- Modify: `src/engine/l2/ir_walk.rs` (every `PCallSite {` construction, ~line 940 and any sibling sites — grep `PCallSite {` in that file)
- Test: inline `#[cfg(test)]` in `src/engine/l2/ir_walk.rs` (or the existing l2 unit-test module if one covers call sites — grep `mod tests` in ir_walk.rs and colocate)

**Interfaces:**
- Produces: `PCallSite.in_statement_position: bool` — `true` iff the call is the outermost expression of a bare call statement (`StmtKind::Call`). serde(skip), excluded from PartialEq. Consumed by Task 5 (d53).

- [ ] **Step 1: Write the failing test**

In `src/engine/l2/ir_walk.rs`'s test module (create `#[cfg(test)] mod stmt_position_tests` at file end if no suitable module exists). Use the same single-routine feature helper the file's other tests use (`ir_features_for_named_routine` from `l2_workspace` is available crate-internally):

```rust
#[cfg(test)]
mod stmt_position_tests {
    use crate::engine::l2::l2_workspace::ir_features_for_named_routine;

    const SRC: &str = r#"
codeunit 50001 T
{
    procedure Caller()
    var
        Ok: Boolean;
    begin
        DoThing();
        if DoThing() then
            Ok := true;
        Ok := DoThing();
    end;

    procedure DoThing(): Boolean
    begin
        exit(true);
    end;
}
"#;

    #[test]
    fn statement_position_set_only_for_bare_call_statements() {
        let (features, _, _) =
            ir_features_for_named_routine(SRC, "Caller", "g", "m", "u").expect("routine");
        let sites: Vec<(&str, bool)> = features
            .call_sites
            .iter()
            .map(|cs| (cs.callee_text.as_str(), cs.in_statement_position))
            .collect();
        // Exactly one DoThing call is a bare statement; the if-condition and the
        // assignment RHS are expression-position (result consumed).
        let stmt_count = sites
            .iter()
            .filter(|(t, p)| t.contains("DoThing") && *p)
            .count();
        let expr_count = sites
            .iter()
            .filter(|(t, p)| t.contains("DoThing") && !*p)
            .count();
        assert_eq!(stmt_count, 1, "sites: {sites:?}");
        assert_eq!(expr_count, 2, "sites: {sites:?}");
    }
}
```

(If `PFeatures`'s call-site container is named differently than `call_sites`, align with the struct — grep `pub struct PFeatures` in `features.rs`.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p al_call_hierarchy --lib stmt_position -- --nocapture`
Expected: FAIL to compile — `in_statement_position` does not exist on `PCallSite`.

- [ ] **Step 3: Add the field (serde-skip, PartialEq-excluded)**

In `src/engine/l2/features.rs`, add to `PCallSite` after `order`:

```rust
    /// True iff this call is the OUTERMOST expression of a bare call statement
    /// (`StmtKind::Call`) — the result is discarded. Expression-position calls
    /// (if-conditions, assignment RHS, arguments) are `false`. Consumed by d53
    /// (ignored TryFunction result).
    ///
    /// INTERNAL-ONLY (`serde(skip)`): never serialized, so every feature-level
    /// golden stays byte-identical; deserialized goldens default it to `false`.
    /// Excluded from PartialEq for the same reason `PVarAssignment.rhs_identifier`
    /// is (baseline vectors deserialize the default and would compare unequal).
    #[serde(skip)]
    pub in_statement_position: bool,
```

`PCallSite` currently derives `PartialEq, Eq`. Convert to a MANUAL impl (mirror `PVarAssignment`'s manual impl at `features.rs:339`) comparing every field EXCEPT `in_statement_position`. Keep `Eq`. The full impl, listing every serialized field explicitly:

```rust
/// MANUAL PartialEq: compares exactly the SERIALIZED L2 contract surface.
/// `in_statement_position` is EXCLUDED — derived, serde-skipped internal data
/// (an L5 input, not part of the L2 shape); baseline vectors deserialize it to
/// the default `false`.
impl PartialEq for PCallSite {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.operation_id == other.operation_id
            && self.callee_text == other.callee_text
            && self.callee == other.callee
            && self.argument_texts == other.argument_texts
            && self.argument_infos == other.argument_infos
            && self.argument_bindings == other.argument_bindings
            && self.loop_stack == other.loop_stack
            && self.source_anchor == other.source_anchor
            && self.result_consumed == other.result_consumed
            && self.object_run_return_used == other.object_run_return_used
            && self.under_asserterror == other.under_asserterror
            && self.control_context == other.control_context
            && self.order == other.order
    }
}
impl Eq for PCallSite {}
```

(Remove `PartialEq, Eq` from the derive list on the struct.)

- [ ] **Step 4: Set the flag at construction**

In `src/engine/l2/ir_walk.rs`, at EVERY `PCallSite {` literal (the main one is ~line 940; grep for others), add:

```rust
                    in_statement_position: self.in_stmt_position,
```

Any construction outside the walker (e.g. test builders elsewhere in the crate that construct `PCallSite` literally — grep `PCallSite {` repo-wide) gets `in_statement_position: false`.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p al_call_hierarchy --lib stmt_position -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Verify no golden churn**

Run: `cargo test --test differential` and `cargo test --test l2_ir`
Expected: PASS with zero divergences (field is serde(skip) + PartialEq-excluded, so L2 vectors/goldens are untouched).

- [ ] **Step 7: Format, lint, commit**

```bash
rustfmt src/engine/l2/features.rs src/engine/l2/ir_walk.rs
cargo clippy --all-targets --all-features
git add src/engine/l2/features.rs src/engine/l2/ir_walk.rs
git commit -m "feat(l2): record statement-position on call sites (d53 substrate)"
```

---

### Task 2: L3Object property forwards — SingleInstance, API-page write surface, object anchor (substrate for d57/d64)

All additive, L3-side only (L3Object is NOT Serialize-derived — no golden risk). Follows the exact pattern `object_subtype`/`page_type` used.

**Files:**
- Modify: `src/engine/l3/l3_workspace.rs` (L3Object struct ~line 33; native assembly ~line 560–660)
- Modify: `src/engine/deps/cross_app_l3.rs:84` (`dep_object_to_l3` — defaults)
- Modify (mechanical — add explicit `None` fields): test constructors in `src/engine/return_summary.rs:941`, `src/engine/l5/fingerprint.rs:262`, `src/engine/l3/receiver_type.rs:1340` and `:1642`, `src/engine/l5/detectors/d17.rs:343`, `src/engine/l5/detectors/d50.rs:472`
- Test: colocated `#[cfg(test)]` in `l3_workspace.rs` (follow whatever workspace-from-source helper its existing tests use)

**Interfaces:**
- Produces on `L3Object`:
  - `single_instance: Option<bool>` — Codeunit `SingleInstance` property (`Some(true)`/`Some(false)` when present, `None` absent).
  - `editable: Option<bool>`, `insert_allowed: Option<bool>`, `modify_allowed: Option<bool>`, `delete_allowed: Option<bool>` — Page-only property booleans.
  - `source_anchor: Option<crate::engine::l2::features::PAnchor>` — the object declaration's own anchor (native path only; dep objects `None`). Consumed by d64 for object-level findings.

- [ ] **Step 1: Write the failing test**

In `l3_workspace.rs`'s existing `#[cfg(test)]` module (grep `mod tests`; follow its fixture mechanism — if it has none, commit a tiny fixture under `tests/fixtures/props/{app.json,src/a.al}` and use `assemble_and_resolve_workspace_default` on it):

```rust
#[test]
fn object_property_forwards_single_instance_and_page_write_surface() {
    // fixture: tests/fixtures/props/ containing app.json (GUID
    // 11111111-0000-0000-0000-0000000000t2 style) and src/a.al:
    //
    //   codeunit 50002 Si
    //   {
    //       SingleInstance = true;
    //   }
    //   page 50002 Api
    //   {
    //       PageType = API;
    //       Editable = false;
    //       InsertAllowed = false;
    //   }
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/props");
    let resolved = assemble_and_resolve_workspace_default(&dir).expect("assemble");
    let cu = resolved.workspace.objects.iter().find(|o| o.name == "Si").unwrap();
    assert_eq!(cu.single_instance, Some(true));
    let pg = resolved.workspace.objects.iter().find(|o| o.name == "Api").unwrap();
    assert_eq!(pg.editable, Some(false));
    assert_eq!(pg.insert_allowed, Some(false));
    assert_eq!(pg.modify_allowed, None);
    assert_eq!(pg.delete_allowed, None);
    assert!(pg.source_anchor.is_some(), "native objects carry their decl anchor");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p al_call_hierarchy --lib object_property_forwards -- --nocapture`
Expected: FAIL to compile — fields missing on `L3Object`.

- [ ] **Step 3: Add fields + harvest**

`L3Object` additions (adjacent to `page_type`, matching neighbor doc-comment style):

```rust
    /// Object `SingleInstance` property (Codeunit only): `Some(true)`/`Some(false)`
    /// when the property is written, `None` when absent. Additive — L3Object is NOT
    /// Serialize-derived into any gate surface. Consumed by d57.
    pub single_instance: Option<bool>,
    /// Page `Editable` / `InsertAllowed` / `ModifyAllowed` / `DeleteAllowed`
    /// property booleans (Page only, `None` when absent). Additive. Consumed by d64.
    pub editable: Option<bool>,
    pub insert_allowed: Option<bool>,
    pub modify_allowed: Option<bool>,
    pub delete_allowed: Option<bool>,
    /// The object DECLARATION's own source anchor (native assembly only; dep
    /// objects `None`). Lets object-level detectors (d64) anchor findings on the
    /// object header instead of borrowing a routine anchor. Additive.
    pub source_anchor: Option<crate::engine::l2::features::PAnchor>,
```

Native harvest, next to the existing `source_table_temporary` harvest (~line 621), using the same `ir_prop` closure and the same exact-"true" boolean convention:

```rust
        let bool_prop = |name: &str| ir_prop(name).map(|v| v.trim().to_lowercase() == "true");
        let single_instance = if object_type == "Codeunit" {
            bool_prop("singleinstance")
        } else {
            None
        };
        let (editable, insert_allowed, modify_allowed, delete_allowed) =
            if object_type == "Page" {
                (
                    bool_prop("editable"),
                    bool_prop("insertallowed"),
                    bool_prop("modifyallowed"),
                    bool_prop("deleteallowed"),
                )
            } else {
                (None, None, None, None)
            };
```

Object anchor: the assembly loop has the IR `ObjectDecl` `o` and the file's `source_unit_id`; build the anchor with the SAME origin→PAnchor conversion the routine anchors use in this file (grep how routine `source_anchor` is built in the assembly and reuse that expression on the object decl's origin). If `ObjectDecl` carries no usable origin span, set `source_anchor: None`, note it in the field doc, and d64 falls back to a line-1 anchor (Task 16 shows the fallback).

Add the six fields to the `L3Object {` literal at `l3_workspace.rs:645`, to `dep_object_to_l3` (`cross_app_l3.rs:84`) as `None` ×6, and to every test constructor listed in Files (all `None`).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p al_call_hierarchy --lib object_property_forwards -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Full-suite sanity, format, lint, commit**

```bash
cargo test --test r4
rustfmt src/engine/l3/l3_workspace.rs src/engine/deps/cross_app_l3.rs src/engine/return_summary.rs src/engine/l5/fingerprint.rs src/engine/l3/receiver_type.rs src/engine/l5/detectors/d17.rs src/engine/l5/detectors/d50.rs
cargo clippy --all-targets --all-features
git add -u src tests
git commit -m "feat(l3): forward SingleInstance + page write-surface properties + object anchor (d57/d64 substrate)"
```

---

### Task 3: Variable scope forward on L3Variable / L3RecordVariable (substrate for d57)

The L2 `PVariableSymbol.scope` (`"local" | "parameter" | "global"`) and `PRecordVariable.scope` exist but the L3 projections drop them. Forward additively.

**Files:**
- Modify: `src/engine/l3/l3_workspace.rs` (`L3Variable` ~line 264; the assembly site mapping `PVariableSymbol` → `L3Variable` — grep `L3Variable {` in src)
- Modify: the `L3RecordVariable` definition/build sites — grep `pub struct L3RecordVariable` and `L3RecordVariable {`; IF it already carries `scope`, skip that half (verify first)
- Test: colocated with Task 2's test module

**Interfaces:**
- Produces: `L3Variable.scope: Option<String>` (`Some("local"|"parameter"|"global")`), same on `L3RecordVariable` if absent today. Consumed by d57.

- [ ] **Step 1: Write the failing test**

Fixture `tests/fixtures/props/src/b.al` (same fixture workspace as Task 2):

```al
codeunit 50003 Sc
{
    var
        GlobalNames: List of [Text];

    procedure P(ParamN: Integer)
    var
        LocalN: Integer;
    begin
        LocalN := ParamN;
        GlobalNames.Add('x');
    end;
}
```

```rust
#[test]
fn variable_scope_forwarded_to_l3() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/props");
    let resolved = assemble_and_resolve_workspace_default(&dir).expect("assemble");
    let r = resolved.workspace.routines.iter().find(|r| r.name == "P").unwrap();
    let scope_of = |n: &str| {
        r.variables
            .iter()
            .find(|v| v.name.eq_ignore_ascii_case(n))
            .and_then(|v| v.scope.clone())
    };
    assert_eq!(scope_of("GlobalNames").as_deref(), Some("global"));
    assert_eq!(scope_of("LocalN").as_deref(), Some("local"));
    assert_eq!(scope_of("ParamN").as_deref(), Some("parameter"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p al_call_hierarchy --lib variable_scope_forwarded -- --nocapture`
Expected: FAIL to compile — no `scope` on `L3Variable`.

- [ ] **Step 3: Add field + forward**

On `L3Variable`:

```rust
    /// Variable scope forwarded from the L2 `PVariableSymbol.scope`:
    /// `"local" | "parameter" | "global"`. `None` only for construction paths
    /// that lack the L2 symbol (defensive). Additive — L3Variable is not
    /// Serialize-derived. Consumed by d57 (global-collection growth).
    pub scope: Option<String>,
```

At the `L3Variable {` construction site(s), forward `scope: Some(v.scope.clone())` from the `PVariableSymbol` (its `scope` field is a plain `String`). Do the equivalent for `L3RecordVariable` from `PRecordVariable.scope: Option<String>` (forward verbatim) IF the field is missing; update any struct-literal constructors that break.

- [ ] **Step 4: Run test to verify it passes; full sanity**

Run: `cargo test -p al_call_hierarchy --lib variable_scope_forwarded -- --nocapture` → PASS
Run: `cargo test --test r4` → PASS (additive; no golden reaches these fields)

- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/engine/l3/l3_workspace.rs
cargo clippy --all-targets --all-features
git add -u src tests
git commit -m "feat(l3): forward variable scope to L3Variable/L3RecordVariable (d57 substrate)"
```

---

## Detector-task boilerplate (referenced by Tasks 4–16)

Every detector task follows this identical mechanical sequence; each task below specifies only its unique content (fixture, detector code, expectations). Where a task says "**Standard wiring steps**", perform exactly:

1. **Fixture:** create `tests/r0-corpus/ws-dNN/{app.json, src/*.al}` with the given content.
2. **Harness entry:** in `tests/r4/r4_differential.rs`, add the fixture to the `WAVE_BCQ` array (create it once in Task 4, after `WAVE_CROSS_APP`, with a matching iteration loop next to the `WAVE_F` loop that pushes into `ported_results`):

```rust
/// BCQuality wave (d52–d64) per-detector fixtures. Each fixture contains both
/// flagged and deliberately-unflagged cases; the golden byte-match pins the
/// exact finding set.
const WAVE_BCQ: &[Smoke] = &[
    // one entry per detector task, e.g.:
    Smoke {
        fixture: "ws-d52",
        wave: "R4-BCQ",
        detectors: &["d52-bulk-write-param-no-temp-guard"],
        ported: true,
        corpus_dir: None,
    },
];
```

and in the test body (after the `WAVE_F_NEGATIVES` loop):

```rust
    // --- R4-BCQ positives (d52–d64, BCQuality wave) ----------------------------
    for smoke in WAVE_BCQ {
        if let Some((matched, count)) =
            run_smoke_entry(smoke, &registered_names, &mut all_divergences)
        {
            ported_results.push((smoke.fixture, matched, count));
        }
    }
```

3. **Neutral negative:** add one entry to the existing `NEGATIVES` array pairing the new detector with the neutral fixture `"ws-e2e"` (follow the array's existing entry shape exactly; if ws-e2e coincidentally triggers the detector, pick another pre-existing neutral fixture that does not contain the pattern and note why).
4. **Module:** create `src/engine/l5/detectors/dNN.rs` with the given code; add `pub mod dNN;` (alphabetical position) in `detectors/mod.rs`.
5. **Registry:** add the `Detector { name, run }` entry in `registered_detectors()` — DEFAULT detectors after the d45 entry (keep d52…d60 in numeric order as they land), OPT-IN after the d51 entry (d61…d64 in numeric order). Update the mod.rs header count comment.
6. **Presets:** add the name to `DEFAULT_DETECTOR_NAMES` or `OPT_IN_DETECTOR_NAMES` in `src/engine/gate/presets.rs` (same position discipline).
7. **Failing run:** `cargo test --test r4 r4_differential::` → FAILS (golden file missing for the new fixture).
8. **Regen + inspect:** `REGEN_TEMP_GOLDENS=1 cargo test --test r4` then `git diff --stat tests/r4-goldens/ && cat tests/r4-goldens/ws-dNN.r4.golden.json`. VERIFY the golden contains exactly the findings the task's "Golden expectations" list — right count, right routine names in ids, right severities. A mismatch is a detector bug: fix the code, regen again. Never commit an uninspected golden. Verify NO OTHER golden changed (`git status tests/`).
9. **Green run:** `cargo test --test r4` → PASS (byte-match + anti-degenerate ≥1 + neutral negative).
10. **Quality gates:** `rustfmt src/engine/l5/detectors/dNN.rs src/engine/l5/detectors/mod.rs src/engine/gate/presets.rs` then `cargo clippy --all-targets --all-features` (clean).
11. **CHANGELOG:** add under `## [Unreleased]` → `### Added`: one line, e.g. `- dNN-<name> detector (BCQuality <rule-slug>): <one-line semantics>.`
12. **Commit:**

```bash
git add src/engine/l5/detectors/dNN.rs src/engine/l5/detectors/mod.rs src/engine/gate/presets.rs tests/r0-corpus/ws-dNN tests/r4-goldens/ws-dNN.r4.golden.json tests/r4/r4_differential.rs CHANGELOG.md
git commit -m "feat(l5): dNN <short-name> detector (BCQuality wave)"
```

---

### Task 4: d52 — bulk write on `var` record parameter without temp proof (DEFAULT)

BCQuality community rule `guard-bulk-operations-with-istemporary`. d33 deliberately skips parameter receivers ("caller responsible"); d52 is the parameter-side complement: a `DeleteAll`/`ModifyAll` on a record PARAMETER with **no temp proof** (not declared `temporary`, no `IsTemporary` entry guard, no G-19 closed-world proof) and **no routine-local filter** is a silent production bulk-write hazard — the helper was almost certainly written for temp buffers.

**Files:**
- Create: `src/engine/l5/detectors/d52.rs`
- Modify: `src/engine/l5/detectors/mod.rs` (promote d33's private `was_filtered_before` to a shared pub(crate) helper; module + registry entries)
- Modify: `src/engine/l5/detectors/d33.rs` (use the promoted helper)
- Modify: `src/engine/gate/presets.rs`, `tests/r4/r4_differential.rs`
- Create: `tests/r0-corpus/ws-d52/{app.json, src/Codeunit.al, src/Tables.al}`, golden `tests/r4-goldens/ws-d52.r4.golden.json`

**Interfaces:**
- Consumes: `is_known_temp`, `entry_temp_guard_receiver`, `ctx.closed_world_temp_params`, `record_filtered_by_call_before` (all existing).
- Produces: shared `pub(crate) fn record_filter_applied_before(ops: &[L3RecordOperation], var_key: &str, bulk_op: &L3RecordOperation) -> bool` in `detectors/mod.rs` (verbatim body of d33's `was_filtered_before`); detector name `"d52-bulk-write-param-no-temp-guard"`.

- [ ] **Step 1: Promote the shared filter helper**

Move d33's `was_filtered_before` (d33.rs:209–233) into `detectors/mod.rs` unchanged as:

```rust
/// Returns true if a `SetRange` / `SetFilter` on `var_key` appears strictly BEFORE
/// `bulk_op` in source order with no intervening `Reset` (which would wipe filters).
/// (Was d33's private `was_filtered_before`; shared with d52.)
pub(crate) fn record_filter_applied_before(
    ops: &[crate::engine::l3::l3_workspace::L3RecordOperation],
    var_key: &str,
    bulk_op: &crate::engine::l3::l3_workspace::L3RecordOperation,
) -> bool {
    let mut filtered = false;
    for other in ops {
        if std::ptr::eq(other, bulk_op) {
            continue;
        }
        if other.record_variable_name.to_lowercase() != var_key {
            continue;
        }
        if !before_anchor(&other.source_anchor, &bulk_op.source_anchor) {
            continue;
        }
        if RECORD_FILTER_OPS.contains(&other.op.as_str()) {
            filtered = true;
        } else if other.op == "Reset" {
            filtered = false;
        }
    }
    filtered
}
```

Update d33 to call `record_filter_applied_before(&routine.record_operations, &var_key, op)` and delete its private copy. Run `cargo test --test r4` — MUST stay green (pure move).

- [ ] **Step 2: Fixture**

`tests/r0-corpus/ws-d52/app.json`:

```json
{
	"id": "11111111-0000-0000-0000-00000000d520",
	"name": "D52 Param Bulk",
	"publisher": "PT",
	"version": "1.0.0.0",
	"dependencies": []
}
```

`tests/r0-corpus/ws-d52/src/Codeunit.al`:

```al
codeunit 50920 "D52 Demo"
{
    // FLAGGED (high): DeleteAll on a var record parameter — no temp proof, no filter.
    procedure NukeBuffer(var Buffer: Record "D52 Buffer")
    begin
        Buffer.DeleteAll();
    end;

    // FLAGGED (medium): ModifyAll variant.
    procedure StampAll(var Buffer: Record "D52 Buffer")
    begin
        Buffer.ModifyAll(Code, 'Y');
    end;

    // NOT FLAGGED: IsTemporary entry guard (G-2) proves tempness.
    procedure CleanupGuarded(var Buffer: Record "D52 Buffer")
    begin
        if not Buffer.IsTemporary() then
            Error('must be temporary');
        Buffer.DeleteAll();
    end;

    // NOT FLAGGED: parameter declared `temporary`.
    procedure CleanupDeclaredTemp(var TempBuffer: Record "D52 Buffer" temporary)
    begin
        TempBuffer.DeleteAll();
    end;

    // NOT FLAGGED: routine-local filter narrows the op (scoped cleanup).
    procedure CleanupFiltered(var Buffer: Record "D52 Buffer")
    begin
        Buffer.SetRange(Code, 'X');
        Buffer.DeleteAll();
    end;

    // NOT FLAGGED: local (non-parameter) receiver — that is d33's territory.
    procedure LocalDelete()
    var
        Buffer: Record "D52 Buffer";
    begin
        Buffer.DeleteAll();
    end;
}
```

`tests/r0-corpus/ws-d52/src/Tables.al`:

```al
table 50920 "D52 Buffer"
{
    fields
    {
        field(1; Code; Code[20]) { }
    }
    keys { key(PK; Code) { } }
}
```

- [ ] **Step 3: Detector module**

`src/engine/l5/detectors/d52.rs`:

```rust
//! D52 — Bulk write on a `var` record PARAMETER without a temp proof.
//! BCQuality community rule `guard-bulk-operations-with-istemporary`.
//!
//! d33 skips parameter receivers (caller-responsible); d52 is the parameter-side
//! complement: `DeleteAll`/`ModifyAll` on a record parameter with NO temp proof
//! (declared `temporary`, the G-2 `IsTemporary` entry guard, or the G-19
//! closed-world proof) and NO routine-local narrowing filter. Such helpers are
//! written for temp buffers; called with a real record they bulk-write the table.
//!
//! Severity: DeleteAll → high, ModifyAll → medium. Confidence: possible
//! (advisory — caller-side filters travel with the record var, so an unfiltered
//! callee op is not PROOF of an unfiltered write).
//!
//! Inert on cross-app contexts only via its normal skips (no resolver join used).

use std::collections::HashMap;

use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::confidence::to_confidence;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::{
    anchor_of, is_known_temp, record_filter_applied_before, record_filtered_by_call_before,
};
use crate::engine::l5::finding::{Evidence, EvidenceStep, Finding, FindingConfidence, FixOption};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorError, DetectorOutput, DetectorStats};

const DETECTOR: &str = "d52-bulk-write-param-no-temp-guard";

const BULK_OPS: &[&str] = &["DeleteAll", "ModifyAll"];

pub fn detect_d52(
    resolved: &L3Resolved,
    ctx: &DetectorContext,
) -> Result<DetectorOutput, DetectorError> {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;
    let mut skipped_known_temp = 0u64;
    let mut skipped_entry_guard = 0u64;
    let mut skipped_closed_world_temp = 0u64;
    let mut skipped_filtered = 0u64;
    let mut skipped_parse_incomplete = 0u64;

    for routine in &ws.routines {
        if !routine.body_available {
            continue;
        }
        if routine.parse_incomplete {
            skipped_parse_incomplete += 1;
            continue;
        }

        // by-name → parameter index for the routine's record PARAMETERS.
        let param_records: HashMap<String, Option<u32>> = routine
            .record_variables
            .iter()
            .filter(|rv| rv.is_parameter)
            .map(|rv| (rv.name.to_lowercase(), rv.parameter_index))
            .collect();

        for op in &routine.record_operations {
            if !BULK_OPS.contains(&op.op.as_str()) {
                continue;
            }
            let var_key = op.record_variable_name.to_lowercase();
            let Some(&param_index) = param_records.get(&var_key) else {
                continue; // non-parameter receivers are d33's territory
            };
            candidates_considered += 1;

            // Temp proofs — a proven-temp buffer bulk-op is the pattern's
            // INTENDED use, never flagged.
            if is_known_temp(op) {
                skipped_known_temp += 1;
                continue;
            }
            if routine.entry_temp_guard_receiver.as_deref() == Some(var_key.as_str()) {
                skipped_entry_guard += 1;
                continue;
            }
            if param_index
                .is_some_and(|pi| ctx.closed_world_temp_params.contains(&(routine.id.clone(), pi)))
            {
                skipped_closed_world_temp += 1;
                continue;
            }
            // A routine-local narrowing filter makes this a scoped cleanup.
            if record_filter_applied_before(&routine.record_operations, &var_key, op)
                || record_filtered_by_call_before(
                    routine,
                    ctx,
                    &op.record_variable_name,
                    &op.source_anchor,
                )
            {
                skipped_filtered += 1;
                continue;
            }

            let table_name = op
                .table_id
                .as_deref()
                .and_then(|tid| ctx.table_by_id.get(tid).map(|t| t.name.clone()))
                .or_else(|| {
                    routine
                        .record_variables
                        .iter()
                        .find(|rv| rv.name.to_lowercase() == var_key)
                        .and_then(|rv| rv.table_name.clone())
                })
                .unwrap_or_else(|| "unknown table".to_string());

            let severity = if op.op == "DeleteAll" { "high" } else { "medium" };
            let confidence: FindingConfidence = to_confidence(&[], "possible");

            let id = format!("d52/{}/{}", routine.id, op.id);
            let mut finding = Finding {
                id: id.clone(),
                root_cause_key: id,
                detector: DETECTOR.to_string(),
                title: format!("{} on unguarded record parameter", op.op),
                root_cause: format!(
                    "{} calls {} on the var record parameter {} ({}) without proving it \
                     temporary (no `temporary` declaration, no IsTemporary entry guard) and \
                     without a local filter — called with a real record this bulk-writes the \
                     whole table.",
                    routine.name, op.op, op.record_variable_name, table_name
                ),
                severity: severity.to_string(),
                confidence,
                primary_location: anchor_of(&op.source_anchor, routine),
                evidence_path: vec![EvidenceStep {
                    routine_id: routine.id.clone(),
                    operation_id: Some(op.id.clone()),
                    callsite_id: None,
                    loop_id: None,
                    source_anchor: anchor_of(&op.source_anchor, routine),
                    note: format!(
                        "{} on parameter {} with no temp proof and no prior filter",
                        op.op, op.record_variable_name
                    ),
                }],
                additional_paths: None,
                affected_objects: vec![routine.object_id.clone()],
                affected_tables: op.table_id.iter().cloned().collect(),
                fix_options: vec![FixOption {
                    description: format!(
                        "Add `if not {0}.IsTemporary() then Error(...)` as the first statement \
                         (or declare the parameter `temporary`), or apply a SetRange/SetFilter \
                         before {1}.",
                        op.record_variable_name, op.op
                    ),
                    safety: "high".to_string(),
                }],
                provenance: vec![Evidence {
                    source: "tree-sitter".to_string(),
                    note: None,
                }],
                actionable_anchor: None,
                fingerprint: None,
                event_kind: None,
                cross_extension_subscribers: None,
            };
            finding.fingerprint = Some(fp_index.fingerprint_of(&finding));
            findings.push(finding);
        }
    }

    findings.sort_by(|a, b| a.id.cmp(&b.id));
    let emitted = findings.len();
    let mut stats = DetectorStats::new(DETECTOR, candidates_considered, emitted);
    stats.add_skip("knownTemp", skipped_known_temp);
    stats.add_skip("entryGuard", skipped_entry_guard);
    stats.add_skip("closedWorldTemp", skipped_closed_world_temp);
    stats.add_skip("filtered", skipped_filtered);
    stats.add_skip("parseIncomplete", skipped_parse_incomplete);
    Ok(DetectorOutput::no_diag(findings, stats))
}
```

- [ ] **Step 4: Standard wiring steps** (boilerplate section above): `pub mod d52;`, registry entry after d45 (`name: "d52-bulk-write-param-no-temp-guard"`, `run: d52::detect_d52`), DEFAULT_DETECTOR_NAMES entry, `WAVE_BCQ` Smoke entry + loop (first task creates the array + loop), NEGATIVES entry vs `ws-e2e`.

- [ ] **Step 5: Golden expectations** (verify at regen — step 8 of boilerplate):
  - Exactly **2 findings**: `NukeBuffer` DeleteAll (severity `high`) and `StampAll` ModifyAll (severity `medium`), both confidence `possible`.
  - `detectorStats` skips: `knownTemp ≥ 2` (guarded + declared-temp), `filtered = 1`. `CleanupGuarded` may land under `entryGuard` instead of `knownTemp` depending on whether G-2 upgraded the op — either is correct; record which.
  - `LocalDelete` absent (non-parameter).

- [ ] **Step 6: Boilerplate steps 9–12** (green run, rustfmt+clippy, CHANGELOG `- d52-bulk-write-param-no-temp-guard detector (BCQuality guard-bulk-operations-with-istemporary): DeleteAll/ModifyAll on a var record parameter without temp proof or local filter.`, commit `feat(l5): d52 bulk-write-param-no-temp-guard detector (BCQuality wave)`).

---

### Task 5: d53 — ignored TryFunction return value (DEFAULT; needs Task 1)

A statement-position call to a `[TryFunction]` swallows the error silently — the single most bug-prone TryFunction misuse.

**Files:**
- Create: `src/engine/l5/detectors/d53.rs`; fixture `tests/r0-corpus/ws-d53/`; golden; standard wiring files.

**Interfaces:**
- Consumes: `PCallSite.in_statement_position` (Task 1), `ctx.resolved_call_edge_by_callsite`, `has_attribute`.
- Produces: detector name `"d53-ignored-tryfunction-result"`. Inert in cross-app contexts (`resolved_call_edge_by_callsite` empty there — documented).

- [ ] **Step 1: Fixture**

`tests/r0-corpus/ws-d53/app.json` (GUID `11111111-0000-0000-0000-00000000d530`, name `"D53 Try Result"`, same shape as Task 4's).

`tests/r0-corpus/ws-d53/src/Codeunit.al`:

```al
codeunit 50921 "D53 Demo"
{
    [TryFunction]
    procedure TryStep()
    begin
        Error('boom');
    end;

    // FLAGGED: statement-position call — a TryFunction failure is silently swallowed.
    procedure IgnoresResult()
    begin
        TryStep();
    end;

    // NOT FLAGGED: result consumed by the if.
    procedure ChecksResult()
    begin
        if not TryStep() then
            Error('step failed');
    end;

    // NOT FLAGGED: under asserterror (deliberate negative-path assertion).
    procedure UnderAssert()
    begin
        asserterror TryStep();
    end;

    // NOT FLAGGED: plain (non-Try) procedure called in statement position.
    procedure PlainStep()
    begin
    end;

    procedure CallsPlain()
    begin
        PlainStep();
    end;
}
```

- [ ] **Step 2: Detector module**

`src/engine/l5/detectors/d53.rs`:

```rust
//! D53 — Ignored `[TryFunction]` return value. BCQuality: a TryFunction called
//! in STATEMENT position discards its implicit Boolean — the error it caught is
//! silently swallowed and execution continues on a failed step.
//!
//! Join: statement-position call site (Task-1 `in_statement_position`) whose
//! RESOLVED callee carries `[TryFunction]`. Skips: `asserterror` scopes
//! (deliberate negative-path assertions). Unresolved callees skip (fail-quiet,
//! advisory precision-first). Inert on the cross-app context
//! (`resolved_call_edge_by_callsite` is empty there).
//!
//! Severity: high. Confidence: likely.

use crate::engine::l3::al_attributes::has_attribute;
use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::confidence::to_confidence;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::anchor_of;
use crate::engine::l5::finding::{Evidence, EvidenceStep, Finding, FindingConfidence, FixOption};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorError, DetectorOutput, DetectorStats};

const DETECTOR: &str = "d53-ignored-tryfunction-result";

pub fn detect_d53(
    resolved: &L3Resolved,
    ctx: &DetectorContext,
) -> Result<DetectorOutput, DetectorError> {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;
    let mut skipped_asserterror = 0u64;
    let mut skipped_result_consumed = 0u64;

    for routine in &ws.routines {
        if !routine.body_available || routine.parse_incomplete {
            continue;
        }
        for cs in &routine.call_sites {
            let Some(edge) = ctx.resolved_call_edge_by_callsite.get(&cs.id) else {
                continue;
            };
            let Some(to) = edge.to.as_deref() else {
                continue;
            };
            let Some(callee) = ctx.routine_by_id.get(to) else {
                continue;
            };
            if !has_attribute(&callee.attributes_parsed, "TryFunction") {
                continue;
            }
            candidates_considered += 1;
            if !cs.in_statement_position {
                skipped_result_consumed += 1;
                continue;
            }
            if cs.under_asserterror == Some(true) {
                skipped_asserterror += 1;
                continue;
            }

            let confidence: FindingConfidence = to_confidence(&[], "likely");
            let id = format!("d53/{}/{}", routine.id, cs.id);
            let mut finding = Finding {
                id: id.clone(),
                root_cause_key: id,
                detector: DETECTOR.to_string(),
                title: "Ignored TryFunction result".to_string(),
                root_cause: format!(
                    "{} calls the TryFunction {} in statement position — the Boolean result \
                     is discarded, so a caught error is silently swallowed and execution \
                     continues past the failed step.",
                    routine.name, callee.name
                ),
                severity: "high".to_string(),
                confidence,
                primary_location: anchor_of(&cs.source_anchor, routine),
                evidence_path: vec![
                    EvidenceStep {
                        routine_id: routine.id.clone(),
                        operation_id: None,
                        callsite_id: Some(cs.id.clone()),
                        loop_id: None,
                        source_anchor: anchor_of(&cs.source_anchor, routine),
                        note: format!("statement-position call to [TryFunction] {}", callee.name),
                    },
                    EvidenceStep {
                        routine_id: callee.id.clone(),
                        operation_id: None,
                        callsite_id: None,
                        loop_id: None,
                        source_anchor: anchor_of(&callee.source_anchor, callee),
                        note: format!("[TryFunction] {}", callee.name),
                    },
                ],
                additional_paths: None,
                affected_objects: vec![routine.object_id.clone()],
                affected_tables: Vec::new(),
                fix_options: vec![FixOption {
                    description: format!(
                        "Consume the result: `if not {}(...) then` handle/surface the failure \
                         (GetLastErrorText), or drop the [TryFunction] attribute if errors \
                         must propagate.",
                        callee.name
                    ),
                    safety: "high".to_string(),
                }],
                provenance: vec![Evidence {
                    source: "tree-sitter".to_string(),
                    note: None,
                }],
                actionable_anchor: None,
                fingerprint: None,
                event_kind: None,
                cross_extension_subscribers: None,
            };
            finding.fingerprint = Some(fp_index.fingerprint_of(&finding));
            findings.push(finding);
        }
    }

    findings.sort_by(|a, b| a.id.cmp(&b.id));
    let emitted = findings.len();
    let mut stats = DetectorStats::new(DETECTOR, candidates_considered, emitted);
    stats.add_skip("resultConsumed", skipped_result_consumed);
    stats.add_skip("asserterror", skipped_asserterror);
    Ok(DetectorOutput::no_diag(findings, stats))
}
```

- [ ] **Step 3: Standard wiring steps.** Name in DEFAULT lists; Smoke entry `detectors: &["d53-ignored-tryfunction-result"]`.

- [ ] **Step 4: Golden expectations:** exactly **1 finding** (`IgnoresResult` → `TryStep`), severity `high`, likely. Stats: `resultConsumed = 1` (ChecksResult), `asserterror = 1` (UnderAssert). `CallsPlain` never a candidate. If the `asserterror TryStep();` call surfaces as expression-position (asserterror wraps an expression), it lands in `resultConsumed` instead — either skip bucket is a correct suppression; record which in the task notes and keep the golden.

- [ ] **Step 5: Boilerplate steps 9–12.** CHANGELOG: `- d53-ignored-tryfunction-result detector: statement-position TryFunction calls silently swallow errors.` Commit `feat(l5): d53 ignored-tryfunction-result detector (BCQuality wave)`.

---

### Task 6: d54 — event published inside a TryFunction cone (DEFAULT)

Unique-to-us call-graph rule: subscribers of an event published (directly or transitively) under a `[TryFunction]` get their errors silenced by the try boundary.

**Files:**
- Create: `src/engine/l5/detectors/d54.rs`; fixture `tests/r0-corpus/ws-d54/`; golden; standard wiring files.

**Interfaces:**
- Consumes: `ctx.graph.edges_by_from` (`CombinedEdge{to, kind}`), `ctx.routine_by_id`, `has_attribute`, `group_and_cap`.
- Produces: detector name `"d54-publish-in-tryfunction-cone"`.

- [ ] **Step 1: Fixture**

`tests/r0-corpus/ws-d54/app.json` (GUID `...d540`, name `"D54 Try Publish"`).

`tests/r0-corpus/ws-d54/src/Codeunit.al`:

```al
codeunit 50922 "D54 Events"
{
    [IntegrationEvent(false, false)]
    procedure OnAfterThing()
    begin
    end;
}

codeunit 50923 "D54 Demo"
{
    // FLAGGED (likely): publisher called directly from a TryFunction body.
    [TryFunction]
    procedure TryDirect()
    var
        Ev: Codeunit "D54 Events";
    begin
        Ev.OnAfterThing();
    end;

    // FLAGGED (possible): publisher reached through a helper.
    [TryFunction]
    procedure TryTransitive()
    begin
        Helper();
    end;

    local procedure Helper()
    var
        Ev: Codeunit "D54 Events";
    begin
        Ev.OnAfterThing();
    end;

    // NOT FLAGGED: same helper from a non-Try caller.
    procedure PlainCaller()
    begin
        Helper();
    end;
}
```

- [ ] **Step 2: Detector module**

`src/engine/l5/detectors/d54.rs`:

```rust
//! D54 — Event published inside a `[TryFunction]` cone. A try boundary swallows
//! errors raised by SUBSCRIBERS of any event published under it — third-party
//! subscriber failures are silenced and their partial writes survive.
//! BCQuality-adjacent; the transitive form is unique to this engine's call graph.
//!
//! For each [TryFunction] routine: BFS over the combined graph's CALL edges
//! (never `event-dispatch` — the cone is the routine's own synchronous closure;
//! never `dynamic` — unresolved). Every reachable `event-publisher` routine is
//! one finding; the BFS parent chain is the evidence path. Publishers are not
//! traversed THROUGH (their bodies are empty declarations).
//!
//! Severity: medium. Confidence: likely when the publisher is called directly
//! from the try body (2-node chain), possible otherwise. Capped at 5 findings
//! per try routine via group_and_cap (skip bucket `cappedPerTryRoutine`).

use std::collections::{HashMap, HashSet, VecDeque};

use crate::engine::l3::al_attributes::has_attribute;
use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::confidence::to_confidence;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::{anchor_of, group_and_cap};
use crate::engine::l5::finding::{Evidence, EvidenceStep, Finding, FindingConfidence, FixOption};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorError, DetectorOutput, DetectorStats};

const DETECTOR: &str = "d54-publish-in-tryfunction-cone";
const MAX_PER_TRY_ROUTINE: usize = 5;

pub fn detect_d54(
    resolved: &L3Resolved,
    ctx: &DetectorContext,
) -> Result<DetectorOutput, DetectorError> {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;
    let mut skipped_no_publisher_reached = 0u64;

    for routine in &ws.routines {
        if !has_attribute(&routine.attributes_parsed, "TryFunction") {
            continue;
        }
        if !routine.body_available || routine.parse_incomplete {
            continue;
        }
        candidates_considered += 1;

        let mut parent: HashMap<&str, &str> = HashMap::new();
        let mut seen: HashSet<&str> = HashSet::new();
        let mut queue: VecDeque<&str> = VecDeque::new();
        seen.insert(routine.id.as_str());
        queue.push_back(routine.id.as_str());
        let mut reached: Vec<&crate::engine::l3::l3_workspace::L3Routine> = Vec::new();

        while let Some(cur) = queue.pop_front() {
            let Some(edges) = ctx.graph.edges_by_from.get(cur) else {
                continue;
            };
            for e in edges {
                // Never cross INTO subscribers (event-dispatch) or through
                // unresolved dynamic edges.
                if e.kind == "event-dispatch" || e.kind == "dynamic" {
                    continue;
                }
                if seen.contains(e.to.as_str()) {
                    continue;
                }
                seen.insert(e.to.as_str());
                parent.insert(e.to.as_str(), cur);
                let Some(target) = ctx.routine_by_id.get(e.to.as_str()) else {
                    continue;
                };
                if target.kind == "event-publisher" {
                    reached.push(target); // do not traverse THROUGH a publisher
                } else {
                    queue.push_back(e.to.as_str());
                }
            }
        }

        if reached.is_empty() {
            skipped_no_publisher_reached += 1;
            continue;
        }

        for publisher in reached {
            // Chain: try routine -> ... -> publisher (via BFS parents).
            let mut chain: Vec<&str> = vec![publisher.id.as_str()];
            let mut cur = publisher.id.as_str();
            while let Some(&p) = parent.get(cur) {
                chain.push(p);
                cur = p;
            }
            chain.reverse();
            let direct = chain.len() == 2;

            let path: Vec<EvidenceStep> = chain
                .iter()
                .filter_map(|rid| ctx.routine_by_id.get(rid))
                .map(|r| EvidenceStep {
                    routine_id: r.id.clone(),
                    operation_id: None,
                    callsite_id: None,
                    loop_id: None,
                    source_anchor: anchor_of(&r.source_anchor, r),
                    note: if r.kind == "event-publisher" {
                        format!("event publisher {}", r.name)
                    } else if has_attribute(&r.attributes_parsed, "TryFunction") {
                        format!("[TryFunction] {}", r.name)
                    } else {
                        r.name.clone()
                    },
                })
                .collect();

            let confidence: FindingConfidence =
                to_confidence(&[], if direct { "likely" } else { "possible" });
            let mut finding = Finding {
                id: format!("d54/{}/{}", routine.id, publisher.id),
                root_cause_key: format!("d54/{}", routine.id),
                detector: DETECTOR.to_string(),
                title: format!(
                    "Event published inside TryFunction cone{}",
                    if direct { "" } else { " (via callee)" }
                ),
                root_cause: format!(
                    "{} is a TryFunction that {} the event publisher {} — errors raised by \
                     subscribers are swallowed by the try boundary, silencing third-party \
                     failures.",
                    routine.name,
                    if direct { "directly calls" } else { "transitively reaches" },
                    publisher.name
                ),
                severity: "medium".to_string(),
                confidence,
                primary_location: anchor_of(&routine.source_anchor, routine),
                evidence_path: path,
                additional_paths: None,
                affected_objects: vec![routine.object_id.clone(), publisher.object_id.clone()],
                affected_tables: Vec::new(),
                fix_options: vec![FixOption {
                    description: "Move the event publish outside the TryFunction boundary, or \
                                  document that subscriber errors are intentionally suppressed \
                                  on this path."
                        .to_string(),
                    safety: "medium".to_string(),
                }],
                provenance: vec![Evidence {
                    source: "tree-sitter".to_string(),
                    note: None,
                }],
                actionable_anchor: None,
                fingerprint: None,
                event_kind: None,
                cross_extension_subscribers: None,
            };
            finding.fingerprint = Some(fp_index.fingerprint_of(&finding));
            findings.push(finding);
        }
    }

    let (mut findings, truncated) =
        group_and_cap(findings, |f| f.root_cause_key.clone(), MAX_PER_TRY_ROUTINE);
    findings.sort_by(|a, b| a.id.cmp(&b.id));
    let emitted = findings.len();
    let mut stats = DetectorStats::new(DETECTOR, candidates_considered, emitted);
    stats.add_skip("noPublisherReached", skipped_no_publisher_reached);
    stats.add_skip("cappedPerTryRoutine", truncated as u64);
    Ok(DetectorOutput::no_diag(findings, stats))
}
```

- [ ] **Step 3: Standard wiring steps.** DEFAULT lists; Smoke `detectors: &["d54-publish-in-tryfunction-cone"]`.

- [ ] **Step 4: Golden expectations:** exactly **2 findings** — `TryDirect` (confidence `likely`, title without suffix) and `TryTransitive` (confidence `possible`, title `... (via callee)`), both severity `medium`, evidence chains of 2 resp. 3 steps. `PlainCaller` produces nothing (not a try routine).

- [ ] **Step 5: Boilerplate steps 9–12.** CHANGELOG: `- d54-publish-in-tryfunction-cone detector: events published under a [TryFunction] silence subscriber errors (call-graph transitive).` Commit `feat(l5): d54 publish-in-tryfunction-cone detector (BCQuality wave)`.

---

### Task 7: d55 — event published inside a loop (DEFAULT)

BCQuality `do-not-publish-events-inside-loops`. d2 covers fan-out-in-loop through the event graph; d55 is the simpler direct check: a call site that RESOLVES to an event-publisher routine with a non-empty `loop_stack`.

**Files:**
- Create: `src/engine/l5/detectors/d55.rs`; fixture `tests/r0-corpus/ws-d55/`; golden; standard wiring files.

**Interfaces:**
- Consumes: `ctx.resolved_call_edge_by_callsite`, `ctx.routine_by_id`, `cs.loop_stack`, `routine.loops`.
- Produces: detector name `"d55-event-publish-in-loop"`. Inert in cross-app contexts (resolver join empty).

- [ ] **Step 1: Fixture**

`tests/r0-corpus/ws-d55/app.json` (GUID `...d550`, name `"D55 Publish Loop"`).

`tests/r0-corpus/ws-d55/src/Codeunit.al`:

```al
codeunit 50924 "D55 Events"
{
    [IntegrationEvent(false, false)]
    procedure OnRowProcessed()
    begin
    end;
}

codeunit 50925 "D55 Demo"
{
    // FLAGGED: publish per iteration — every subscriber runs once per row.
    procedure PublishInLoop()
    var
        Item: Record "D55 Item";
        Ev: Codeunit "D55 Events";
    begin
        if Item.FindSet() then
            repeat
                Ev.OnRowProcessed();
            until Item.Next() = 0;
    end;

    // NOT FLAGGED: publish once after the loop.
    procedure PublishAfterLoop()
    var
        Item: Record "D55 Item";
        Ev: Codeunit "D55 Events";
    begin
        if Item.FindSet() then
            repeat
            until Item.Next() = 0;
        Ev.OnRowProcessed();
    end;
}
```

`tests/r0-corpus/ws-d55/src/Tables.al`:

```al
table 50925 "D55 Item"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }
    keys { key(PK; "No.") { } }
}
```

- [ ] **Step 2: Detector module**

`src/engine/l5/detectors/d55.rs`:

```rust
//! D55 — Direct event publish inside a loop. BCQuality
//! `do-not-publish-events-inside-loops`: each iteration dispatches EVERY
//! subscriber — cost is subscribers × iterations and grows as third parties
//! subscribe. d2 covers transitive fan-out-in-loop; d55 is the direct,
//! declaration-independent form (fires even with zero current subscribers —
//! the publish point itself is the hazard).
//!
//! Join: call site with non-empty loop_stack whose RESOLVED callee has
//! kind == "event-publisher". Severity: high when loop depth ≥ 2, else medium.
//! Confidence: likely. Inert on the cross-app context (resolver join empty).

use std::collections::HashMap;

use crate::engine::l2::features::PLoop;
use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::confidence::to_confidence;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::anchor_of;
use crate::engine::l5::finding::{Evidence, EvidenceStep, Finding, FindingConfidence, FixOption};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorError, DetectorOutput, DetectorStats};

const DETECTOR: &str = "d55-event-publish-in-loop";

pub fn detect_d55(
    resolved: &L3Resolved,
    ctx: &DetectorContext,
) -> Result<DetectorOutput, DetectorError> {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;
    let mut skipped_not_in_loop = 0u64;

    for routine in &ws.routines {
        if !routine.body_available || routine.parse_incomplete {
            continue;
        }
        let loop_by_id: HashMap<&str, &PLoop> =
            routine.loops.iter().map(|l| (l.id.as_str(), l)).collect();

        for cs in &routine.call_sites {
            let Some(edge) = ctx.resolved_call_edge_by_callsite.get(&cs.id) else {
                continue;
            };
            let Some(to) = edge.to.as_deref() else {
                continue;
            };
            let Some(callee) = ctx.routine_by_id.get(to) else {
                continue;
            };
            if callee.kind != "event-publisher" {
                continue;
            }
            candidates_considered += 1;
            let Some(rep_loop_id) = cs.loop_stack.last() else {
                skipped_not_in_loop += 1;
                continue;
            };
            let Some(loop_info) = loop_by_id.get(rep_loop_id.as_str()) else {
                skipped_not_in_loop += 1;
                continue;
            };

            let severity = if cs.loop_stack.len() >= 2 { "high" } else { "medium" };
            let confidence: FindingConfidence = to_confidence(&[], "likely");
            let id = format!("d55/{}/{}/{}", routine.id, loop_info.id, cs.id);
            let mut finding = Finding {
                id: id.clone(),
                root_cause_key: format!("d55/{}/{}", routine.id, loop_info.id),
                detector: DETECTOR.to_string(),
                title: "Event published inside loop".to_string(),
                root_cause: format!(
                    "{} publishes {} inside a {} loop — every subscriber runs once per \
                     iteration, and the cost grows as third parties subscribe.",
                    routine.name, callee.name, loop_info.loop_type
                ),
                severity: severity.to_string(),
                confidence,
                primary_location: anchor_of(&cs.source_anchor, routine),
                evidence_path: vec![
                    EvidenceStep {
                        routine_id: routine.id.clone(),
                        operation_id: None,
                        callsite_id: None,
                        loop_id: Some(loop_info.id.clone()),
                        source_anchor: anchor_of(&loop_info.source_anchor, routine),
                        note: format!("{} loop", loop_info.loop_type),
                    },
                    EvidenceStep {
                        routine_id: routine.id.clone(),
                        operation_id: None,
                        callsite_id: Some(cs.id.clone()),
                        loop_id: Some(loop_info.id.clone()),
                        source_anchor: anchor_of(&cs.source_anchor, routine),
                        note: format!("publishes {}", callee.name),
                    },
                ],
                additional_paths: None,
                affected_objects: vec![routine.object_id.clone(), callee.object_id.clone()],
                affected_tables: Vec::new(),
                fix_options: vec![FixOption {
                    description: "Accumulate the per-row data and publish ONE event after the \
                                  loop (pass a collection/buffer), or document why per-row \
                                  dispatch is required."
                        .to_string(),
                    safety: "medium".to_string(),
                }],
                provenance: vec![Evidence {
                    source: "tree-sitter".to_string(),
                    note: None,
                }],
                actionable_anchor: None,
                fingerprint: None,
                event_kind: None,
                cross_extension_subscribers: None,
            };
            finding.fingerprint = Some(fp_index.fingerprint_of(&finding));
            findings.push(finding);
        }
    }

    findings.sort_by(|a, b| a.id.cmp(&b.id));
    let emitted = findings.len();
    let mut stats = DetectorStats::new(DETECTOR, candidates_considered, emitted);
    stats.add_skip("notInLoop", skipped_not_in_loop);
    Ok(DetectorOutput::no_diag(findings, stats))
}
```

- [ ] **Step 3: Standard wiring steps.** DEFAULT lists; Smoke `detectors: &["d55-event-publish-in-loop"]`.

- [ ] **Step 4: Golden expectations:** exactly **1 finding** (`PublishInLoop`, severity `medium`, likely); stats `notInLoop = 1` (`PublishAfterLoop`).

- [ ] **Step 5: Boilerplate steps 9–12.** CHANGELOG: `- d55-event-publish-in-loop detector (BCQuality do-not-publish-events-inside-loops).` Commit `feat(l5): d55 event-publish-in-loop detector (BCQuality wave)`.

---

### Task 8: d56 — record cloned before Modify/Delete in a loop (DEFAULT)

BCQuality `avoid-cloning-records-before-modify-delete-in-loops`: `Copy := Cursor; Copy.Modify()` inside the cursor's loop costs an extra SQL round-trip per row.

**Files:**
- Create: `src/engine/l5/detectors/d56.rs`; fixture `tests/r0-corpus/ws-d56/`; golden; standard wiring files.

**Interfaces:**
- Consumes: `routine.var_assignments` (`rhs_identifier` whole-record copies), `routine.record_operations` (`loop_stack`), `routine.loops`, `before_anchor`.
- Produces: detector name `"d56-clone-before-write-in-loop"`; local helper `anchor_within(inner, outer) -> bool`.

- [ ] **Step 1: Fixture**

`tests/r0-corpus/ws-d56/app.json` (GUID `...d560`, name `"D56 Clone Loop"`).

`tests/r0-corpus/ws-d56/src/Codeunit.al`:

```al
codeunit 50926 "D56 Demo"
{
    // FLAGGED: clone of the loop cursor written back inside the loop.
    procedure CloneAndModify()
    var
        Cust: Record "D56 Customer";
        CustCopy: Record "D56 Customer";
    begin
        if Cust.FindSet() then
            repeat
                CustCopy := Cust;
                CustCopy.Name := 'X';
                CustCopy.Modify();
            until Cust.Next() = 0;
    end;

    // NOT FLAGGED: cursor modified directly (d10's territory, not d56's).
    procedure DirectModify()
    var
        Cust: Record "D56 Customer";
    begin
        if Cust.FindSet() then
            repeat
                Cust.Name := 'X';
                Cust.Modify();
            until Cust.Next() = 0;
    end;

    // NOT FLAGGED: copy taken outside any loop.
    procedure CopyOutsideLoop()
    var
        Cust: Record "D56 Customer";
        CustCopy: Record "D56 Customer";
    begin
        Cust.FindFirst();
        CustCopy := Cust;
        CustCopy.Modify();
    end;

    // NOT FLAGGED: clone inside the loop but never written back.
    procedure CloneReadOnly()
    var
        Cust: Record "D56 Customer";
        CustCopy: Record "D56 Customer";
    begin
        if Cust.FindSet() then
            repeat
                CustCopy := Cust;
            until Cust.Next() = 0;
    end;
}
```

`tests/r0-corpus/ws-d56/src/Tables.al`:

```al
table 50926 "D56 Customer"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; Name; Text[100]) { }
    }
    keys { key(PK; "No.") { } }
}
```

- [ ] **Step 2: Detector module**

`src/engine/l5/detectors/d56.rs`:

```rust
//! D56 — Record cloned before Modify/Delete inside a loop. BCQuality
//! `avoid-cloning-records-before-modify-delete-in-loops`: `Copy := Cursor;
//! Copy.Modify();` per iteration re-reads/re-writes the row the cursor already
//! holds — an extra SQL round-trip per row. Modify the cursor directly (or use
//! ModifyAll / a temp buffer).
//!
//! Join (all intraprocedural, exact):
//!  - a whole-record copy `lhs := rhs` (PVarAssignment.rhs_identifier — set ONLY
//!    for bare-identifier-to-bare-identifier copies) between two RECORD vars,
//!  - the assignment sits inside a loop (innermost containing PLoop by anchor),
//!  - a Modify/Delete on the CLONE, in the SAME loop (op.loop_stack), AFTER the copy,
//!  - the SOURCE is a live cursor (has FindSet/Find/FindFirst/Next ops in the routine).
//!
//! Severity: medium. Confidence: likely.

use std::collections::HashSet;

use crate::engine::l2::features::{PAnchor, PLoop};
use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::confidence::to_confidence;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::{anchor_of, before_anchor};
use crate::engine::l5::finding::{Evidence, EvidenceStep, Finding, FindingConfidence, FixOption};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorError, DetectorOutput, DetectorStats};

const DETECTOR: &str = "d56-clone-before-write-in-loop";

const CURSOR_OPS: &[&str] = &["FindSet", "Find", "FindFirst", "Next"];
const WRITE_BACK_OPS: &[&str] = &["Modify", "Delete"];

/// Anchor containment: `inner` fully inside `outer`.
fn anchor_within(inner: &PAnchor, outer: &PAnchor) -> bool {
    let starts_ok = outer.start_line < inner.start_line
        || (outer.start_line == inner.start_line && outer.start_column <= inner.start_column);
    let ends_ok = inner.end_line < outer.end_line
        || (inner.end_line == outer.end_line && inner.end_column <= outer.end_column);
    starts_ok && ends_ok
}

pub fn detect_d56(
    resolved: &L3Resolved,
    _ctx: &DetectorContext,
) -> Result<DetectorOutput, DetectorError> {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;
    let mut skipped_no_write_back = 0u64;
    let mut skipped_source_not_cursor = 0u64;

    for routine in &ws.routines {
        if !routine.body_available || routine.parse_incomplete {
            continue;
        }
        if routine.loops.is_empty() || routine.var_assignments.is_empty() {
            continue;
        }
        let record_names: HashSet<String> = routine
            .record_variables
            .iter()
            .map(|rv| rv.name.to_lowercase())
            .collect();

        for asg in &routine.var_assignments {
            let Some(rhs_lc) = asg.rhs_identifier.as_deref() else {
                continue;
            };
            // Whole-record copy between two record vars only.
            if !record_names.contains(rhs_lc) || !record_names.contains(&asg.lhs_name) {
                continue;
            }
            // Innermost loop containing the assignment (assignments carry no
            // loop_stack; containment is by anchor).
            let Some(lp) = routine
                .loops
                .iter()
                .filter(|l| anchor_within(&asg.source_anchor, &l.source_anchor))
                .max_by_key(|l| (l.source_anchor.start_line, l.source_anchor.start_column))
            else {
                continue;
            };
            candidates_considered += 1;

            // Written back inside the SAME loop, after the copy.
            let write = routine.record_operations.iter().find(|op| {
                WRITE_BACK_OPS.contains(&op.op.as_str())
                    && op.record_variable_name.to_lowercase() == asg.lhs_name
                    && op.loop_stack.iter().any(|id| id == &lp.id)
                    && before_anchor(&asg.source_anchor, &op.source_anchor)
            });
            let Some(write) = write else {
                skipped_no_write_back += 1;
                continue;
            };
            // The copy SOURCE must be a live cursor.
            let src_is_cursor = routine.record_operations.iter().any(|op| {
                CURSOR_OPS.contains(&op.op.as_str())
                    && op.record_variable_name.to_lowercase() == rhs_lc
            });
            if !src_is_cursor {
                skipped_source_not_cursor += 1;
                continue;
            }

            let confidence: FindingConfidence = to_confidence(&[], "likely");
            let id = format!("d56/{}/{}/{}", routine.id, lp.id, write.id);
            let mut finding = Finding {
                id: id.clone(),
                root_cause_key: format!("d56/{}/{}", routine.id, lp.id),
                detector: DETECTOR.to_string(),
                title: format!("Record cloned before {} in loop", write.op),
                root_cause: format!(
                    "{} copies the loop cursor {} into {} and calls {} on the copy inside \
                     the loop — an extra SQL round-trip per row; the cursor already holds \
                     the row.",
                    routine.name, rhs_lc, asg.lhs_name, write.op
                ),
                severity: "medium".to_string(),
                confidence,
                primary_location: anchor_of(&write.source_anchor, routine),
                evidence_path: vec![
                    EvidenceStep {
                        routine_id: routine.id.clone(),
                        operation_id: None,
                        callsite_id: None,
                        loop_id: Some(lp.id.clone()),
                        source_anchor: anchor_of(&asg.source_anchor, routine),
                        note: format!("clone {} := {} inside {} loop", asg.lhs_name, rhs_lc, lp.loop_type),
                    },
                    EvidenceStep {
                        routine_id: routine.id.clone(),
                        operation_id: Some(write.id.clone()),
                        callsite_id: None,
                        loop_id: Some(lp.id.clone()),
                        source_anchor: anchor_of(&write.source_anchor, routine),
                        note: format!("{} on the clone", write.op),
                    },
                ],
                additional_paths: None,
                affected_objects: vec![routine.object_id.clone()],
                affected_tables: write.table_id.iter().cloned().collect(),
                fix_options: vec![FixOption {
                    description: format!(
                        "Call {} on the cursor ({}) directly, or restructure to a set-based \
                         write (ModifyAll/DeleteAll) outside the loop.",
                        write.op, rhs_lc
                    ),
                    safety: "medium".to_string(),
                }],
                provenance: vec![Evidence {
                    source: "tree-sitter".to_string(),
                    note: None,
                }],
                actionable_anchor: None,
                fingerprint: None,
                event_kind: None,
                cross_extension_subscribers: None,
            };
            finding.fingerprint = Some(fp_index.fingerprint_of(&finding));
            findings.push(finding);
        }
    }

    findings.sort_by(|a, b| a.id.cmp(&b.id));
    let emitted = findings.len();
    let mut stats = DetectorStats::new(DETECTOR, candidates_considered, emitted);
    stats.add_skip("noWriteBack", skipped_no_write_back);
    stats.add_skip("sourceNotCursor", skipped_source_not_cursor);
    Ok(DetectorOutput::no_diag(findings, stats))
}
```

- [ ] **Step 3: Standard wiring steps.** DEFAULT lists; Smoke `detectors: &["d56-clone-before-write-in-loop"]`.

- [ ] **Step 4: Golden expectations:** exactly **1 finding** (`CloneAndModify`, severity `medium`, likely). Stats: `noWriteBack = 1` (`CloneReadOnly`); `CopyOutsideLoop` never a candidate (no containing loop); `DirectModify` never a candidate (no clone assignment).

- [ ] **Step 5: Boilerplate steps 9–12.** CHANGELOG: `- d56-clone-before-write-in-loop detector (BCQuality avoid-cloning-records-before-modify-delete-in-loops).` Commit `feat(l5): d56 clone-before-write-in-loop detector (BCQuality wave)`.

---

### Task 9: d57 — growing globals in SingleInstance subscribers (DEFAULT; needs Tasks 2+3)

Session-lifetime memory leak: a SingleInstance codeunit lives for the whole session; a subscriber appending to a global collection (or inserting into a global temp record) with no clearing path anywhere in the object grows unboundedly.

**Files:**
- Create: `src/engine/l5/detectors/d57.rs`; fixture `tests/r0-corpus/ws-d57/`; golden; standard wiring files.

**Interfaces:**
- Consumes: `L3Object.single_instance` (Task 2), `L3Variable.scope` / `L3RecordVariable.scope` (Task 3), `PCallee`, `is_known_temp`.
- Produces: detector name `"d57-singleinstance-growing-state"`.

- [ ] **Step 1: Fixture**

`tests/r0-corpus/ws-d57/app.json` (GUID `...d570`, name `"D57 Single Instance"`).

`tests/r0-corpus/ws-d57/src/Codeunit.al`:

```al
codeunit 50927 "D57 Events"
{
    [IntegrationEvent(false, false)]
    procedure OnThing()
    begin
    end;
}

codeunit 50928 "D57 Leaky"
{
    SingleInstance = true;

    var
        SeenNames: List of [Text];
        TempLog: Record "D57 Log" temporary;

    // FLAGGED ×2: unbounded growth of session-lifetime state (list Add + temp Insert).
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"D57 Events", 'OnThing', '', false, false)]
    local procedure OnThingSub()
    begin
        SeenNames.Add('x');
        TempLog.Init();
        TempLog.Insert();
    end;
}

codeunit 50929 "D57 Drained"
{
    SingleInstance = true;

    var
        Pending: List of [Text];

    // NOT FLAGGED: a clearing path exists in the same object.
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"D57 Events", 'OnThing', '', false, false)]
    local procedure OnThingSub()
    begin
        Pending.Add('x');
    end;

    procedure Drain()
    begin
        Clear(Pending);
    end;
}

codeunit 50930 "D57 NotSingle"
{
    var
        Names: List of [Text];

    // NOT FLAGGED: object is not SingleInstance.
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"D57 Events", 'OnThing', '', false, false)]
    local procedure OnThingSub()
    begin
        Names.Add('x');
    end;
}
```

`tests/r0-corpus/ws-d57/src/Tables.al`:

```al
table 50927 "D57 Log"
{
    fields
    {
        field(1; Id; Integer) { }
    }
    keys { key(PK; Id) { } }
}
```

- [ ] **Step 2: Detector module**

`src/engine/l5/detectors/d57.rs`:

```rust
//! D57 — Growing globals in SingleInstance subscribers. A SingleInstance
//! codeunit's globals live for the SESSION; an event subscriber that appends to
//! a global collection (`List`/`Dictionary` `.Add`/`.Insert`/`.AddRange`) or
//! inserts into a global TEMP record, with NO clearing path anywhere in the
//! object (`Clear`/`Remove*` member or bare `Clear(<g>)`, `Delete`/`DeleteAll`
//! for records), grows unboundedly — a session-lifetime memory leak.
//!
//! Every uncertainty (non-global receiver, unknown scope, cleared-somewhere)
//! SKIPS — advisory precision-first. Severity: medium. Confidence: possible.

use std::collections::{HashMap, HashSet};

use crate::engine::l2::features::PCallee;
use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::confidence::to_confidence;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::{anchor_of, is_known_temp};
use crate::engine::l5::finding::{Evidence, EvidenceStep, Finding, FindingConfidence, FixOption};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorError, DetectorOutput, DetectorStats};

const DETECTOR: &str = "d57-singleinstance-growing-state";

const GROW_METHODS: &[&str] = &["Add", "Insert", "AddRange"];
const CLEAR_METHODS: &[&str] = &["Clear", "Remove", "RemoveAt", "RemoveRange", "DeleteAll", "Delete"];

pub fn detect_d57(
    resolved: &L3Resolved,
    _ctx: &DetectorContext,
) -> Result<DetectorOutput, DetectorError> {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;
    let mut skipped_cleared_in_object = 0u64;
    let mut skipped_not_collection = 0u64;

    // SingleInstance codeunit object ids.
    let si_objects: HashSet<&str> = ws
        .objects
        .iter()
        .filter(|o| o.object_type == "Codeunit" && o.single_instance == Some(true))
        .map(|o| o.id.as_str())
        .collect();
    if si_objects.is_empty() {
        let stats = DetectorStats::new(DETECTOR, 0, 0);
        return Ok(DetectorOutput::no_diag(findings, stats));
    }

    // Per SingleInstance object: receiver names (lowercased) with ANY clearing
    // signal anywhere in the object.
    let mut cleared_by_object: HashMap<&str, HashSet<String>> = HashMap::new();
    for r in &ws.routines {
        if !si_objects.contains(r.object_id.as_str()) {
            continue;
        }
        let set = cleared_by_object.entry(r.object_id.as_str()).or_default();
        for cs in &r.call_sites {
            match &cs.callee {
                PCallee::Member { receiver, method }
                    if CLEAR_METHODS.iter().any(|m| m.eq_ignore_ascii_case(method)) =>
                {
                    set.insert(receiver.to_lowercase());
                }
                PCallee::Bare { name } if name.eq_ignore_ascii_case("Clear") => {
                    for b in &cs.argument_bindings {
                        if let Some(v) = &b.source_variable_name {
                            set.insert(v.clone()); // stored lowercased
                        }
                    }
                }
                _ => {}
            }
        }
        for op in &r.record_operations {
            if op.op == "DeleteAll" || op.op == "Delete" {
                set.insert(op.record_variable_name.to_lowercase());
            }
        }
    }

    for routine in &ws.routines {
        if routine.kind != "event-subscriber" {
            continue;
        }
        if !si_objects.contains(routine.object_id.as_str()) {
            continue;
        }
        if !routine.body_available || routine.parse_incomplete {
            continue;
        }
        let cleared = cleared_by_object.get(routine.object_id.as_str());
        let is_cleared = |name_lc: &str| cleared.is_some_and(|s| s.contains(name_lc));

        // (a) Global List/Dictionary growth.
        for cs in &routine.call_sites {
            let PCallee::Member { receiver, method } = &cs.callee else {
                continue;
            };
            if !GROW_METHODS.iter().any(|m| m.eq_ignore_ascii_case(method)) {
                continue;
            }
            let recv_lc = receiver.to_lowercase();
            let Some(v) = routine.variables.iter().find(|v| {
                v.name.to_lowercase() == recv_lc && v.scope.as_deref() == Some("global")
            }) else {
                continue;
            };
            let ty = v.declared_type.to_lowercase();
            if !(ty.starts_with("list of") || ty.starts_with("dictionary of")) {
                skipped_not_collection += 1;
                continue;
            }
            candidates_considered += 1;
            if is_cleared(&recv_lc) {
                skipped_cleared_in_object += 1;
                continue;
            }
            findings.push(build_finding(
                &fp_index,
                routine,
                &cs.id,
                anchor_of(&cs.source_anchor, routine),
                &format!(
                    "{} is an event subscriber in a SingleInstance codeunit appending to \
                     the global {} {} — no clearing path exists in the object, so the \
                     collection grows for the whole session.",
                    routine.name, v.declared_type, receiver
                ),
                receiver,
            ));
        }

        // (b) Global temp-record growth.
        for op in &routine.record_operations {
            if op.op != "Insert" {
                continue;
            }
            let var_lc = op.record_variable_name.to_lowercase();
            let Some(rv) = routine
                .record_variables
                .iter()
                .find(|rv| rv.name.to_lowercase() == var_lc)
            else {
                continue;
            };
            if rv.scope.as_deref() != Some("global") {
                continue;
            }
            // Physical global inserts are transaction detectors' territory —
            // d57 only tracks in-memory growth.
            if !is_known_temp(op) {
                continue;
            }
            candidates_considered += 1;
            if is_cleared(&var_lc) {
                skipped_cleared_in_object += 1;
                continue;
            }
            findings.push(build_finding(
                &fp_index,
                routine,
                &op.id,
                anchor_of(&op.source_anchor, routine),
                &format!(
                    "{} is an event subscriber in a SingleInstance codeunit inserting into \
                     the global temporary record {} — no Delete/DeleteAll exists in the \
                     object, so the buffer grows for the whole session.",
                    routine.name, op.record_variable_name
                ),
                &op.record_variable_name,
            ));
        }
    }

    findings.sort_by(|a, b| a.id.cmp(&b.id));
    let emitted = findings.len();
    let mut stats = DetectorStats::new(DETECTOR, candidates_considered, emitted);
    stats.add_skip("clearedInObject", skipped_cleared_in_object);
    stats.add_skip("notCollection", skipped_not_collection);
    Ok(DetectorOutput::no_diag(findings, stats))
}

fn build_finding(
    fp_index: &FingerprintIndex,
    routine: &crate::engine::l3::l3_workspace::L3Routine,
    site_id: &str,
    anchor: crate::engine::l5::finding::SourceAnchor,
    root_cause: &str,
    var_name: &str,
) -> Finding {
    let confidence: FindingConfidence = to_confidence(&[], "possible");
    let id = format!("d57/{}/{}", routine.id, site_id);
    let mut finding = Finding {
        id: id.clone(),
        root_cause_key: format!("d57/{}/{}", routine.id, var_name.to_lowercase()),
        detector: DETECTOR.to_string(),
        title: "Growing global state in SingleInstance subscriber".to_string(),
        root_cause: root_cause.to_string(),
        severity: "medium".to_string(),
        confidence,
        primary_location: anchor.clone(),
        evidence_path: vec![EvidenceStep {
            routine_id: routine.id.clone(),
            operation_id: None,
            callsite_id: None,
            loop_id: None,
            source_anchor: anchor,
            note: format!("unbounded append to global {var_name}"),
        }],
        additional_paths: None,
        affected_objects: vec![routine.object_id.clone()],
        affected_tables: Vec::new(),
        fix_options: vec![FixOption {
            description: format!(
                "Bound the growth: clear/drain {var_name} (Clear/Remove/DeleteAll) on a \
                 defined lifecycle point, or replace the session-lifetime cache with a \
                 keyed lookup that overwrites instead of appending."
            ),
            safety: "medium".to_string(),
        }],
        provenance: vec![Evidence {
            source: "tree-sitter".to_string(),
            note: None,
        }],
        actionable_anchor: None,
        fingerprint: None,
        event_kind: None,
        cross_extension_subscribers: None,
    };
    finding.fingerprint = Some(fp_index.fingerprint_of(&finding));
    finding
}
```

(`EvidenceStep.callsite_id`/`operation_id` are both `None` in the shared builder because the two shapes differ; if review prefers precise ids, split the builder — cosmetic either way.)

- [ ] **Step 3: Standard wiring steps.** DEFAULT lists; Smoke `detectors: &["d57-singleinstance-growing-state"]`.

- [ ] **Step 4: Golden expectations:** exactly **2 findings**, both in `"D57 Leaky".OnThingSub` (one for `SeenNames.Add`, one for `TempLog.Insert`), severity `medium`, possible. Stats: `clearedInObject = 1` (`"D57 Drained"`). `"D57 NotSingle"` never a candidate.

- [ ] **Step 5: Boilerplate steps 9–12.** CHANGELOG: `- d57-singleinstance-growing-state detector: unbounded global collection/temp-record growth in SingleInstance subscribers.` Commit `feat(l5): d57 singleinstance-growing-state detector (BCQuality wave)`.

---

### Task 10: d58 — Query filter set after Open (DEFAULT)

BCQuality `set-query-filters-before-open`: `SetFilter`/`SetRange` on an already-open query is ignored by the running dataset.

**Files:**
- Create: `src/engine/l5/detectors/d58.rs`; fixture `tests/r0-corpus/ws-d58/`; golden; standard wiring files.

**Interfaces:**
- Consumes: `routine.variables` (`declared_type` starting `Query`), member call sites, `before_anchor`.
- Produces: detector name `"d58-query-filter-after-open"`.

- [ ] **Step 1: Fixture**

`tests/r0-corpus/ws-d58/app.json` (GUID `...d580`, name `"D58 Query Filters"`).

`tests/r0-corpus/ws-d58/src/Objects.al`:

```al
query 50931 "D58 Items"
{
    elements
    {
        dataitem(Item; "D58 Item")
        {
            column(No; "No.") { }
        }
    }
}

codeunit 50932 "D58 Demo"
{
    // FLAGGED: filter applied after Open is ignored by the open dataset.
    procedure FilterAfterOpen()
    var
        Q: Query "D58 Items";
    begin
        Q.Open();
        Q.SetFilter(No, '1000..');
    end;

    // NOT FLAGGED: filter before Open.
    procedure FilterBeforeOpen()
    var
        Q: Query "D58 Items";
    begin
        Q.SetFilter(No, '1000..');
        Q.Open();
    end;

    // NOT FLAGGED: Close re-arms filtering; filter lands before the re-Open.
    procedure CloseThenFilter()
    var
        Q: Query "D58 Items";
    begin
        Q.Open();
        Q.Close();
        Q.SetFilter(No, '1000..');
        Q.Open();
    end;
}

table 50931 "D58 Item"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }
    keys { key(PK; "No.") { } }
}
```

(If the grammar/fixture pipeline chokes on the `query` object shape, verify with `tree-sitter parse` per CLAUDE.md; the detector itself only needs the CODEUNIT half — the query object can degrade to any parseable stub as long as `Q: Query "D58 Items"` variables parse.)

- [ ] **Step 2: Detector module**

`src/engine/l5/detectors/d58.rs`:

```rust
//! D58 — Query filter set after `Open()`. BCQuality `set-query-filters-before-open`:
//! the running dataset snapshots filters at Open; a later SetFilter/SetRange is
//! silently ignored until the next Open. `Close()` re-arms filtering.
//!
//! Intraprocedural, straight-line source order (branching ignored — the same
//! convention d33's filter scan uses). Per query-typed variable, walk its
//! member calls in anchor order tracking open-state; flag each
//! SetFilter/SetRange while open. Severity: medium. Confidence: likely.

use crate::engine::l2::features::PCallee;
use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::confidence::to_confidence;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::anchor_of;
use crate::engine::l5::finding::{Evidence, EvidenceStep, Finding, FindingConfidence, FixOption};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorError, DetectorOutput, DetectorStats};

const DETECTOR: &str = "d58-query-filter-after-open";

pub fn detect_d58(
    resolved: &L3Resolved,
    _ctx: &DetectorContext,
) -> Result<DetectorOutput, DetectorError> {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;
    let mut skipped_before_open = 0u64;

    for routine in &ws.routines {
        if !routine.body_available || routine.parse_incomplete {
            continue;
        }
        // Query-typed variables (params/locals/globals).
        let query_vars: Vec<String> = routine
            .variables
            .iter()
            .filter(|v| v.declared_type.trim_start().to_lowercase().starts_with("query"))
            .map(|v| v.name.to_lowercase())
            .collect();
        if query_vars.is_empty() {
            continue;
        }

        for qv in &query_vars {
            // (anchor-ordered) events on this receiver: Open / Close / SetFilter/SetRange.
            let mut events: Vec<(&crate::engine::l2::features::PCallSite, &'static str)> =
                Vec::new();
            for cs in &routine.call_sites {
                let PCallee::Member { receiver, method } = &cs.callee else {
                    continue;
                };
                if receiver.to_lowercase() != *qv {
                    continue;
                }
                let ev = if method.eq_ignore_ascii_case("Open") {
                    "open"
                } else if method.eq_ignore_ascii_case("Close") {
                    "close"
                } else if method.eq_ignore_ascii_case("SetFilter")
                    || method.eq_ignore_ascii_case("SetRange")
                {
                    "filter"
                } else {
                    continue;
                };
                events.push((cs, ev));
            }
            events.sort_by_key(|(cs, _)| {
                (cs.source_anchor.start_line, cs.source_anchor.start_column)
            });

            let mut is_open = false;
            for (cs, ev) in events {
                match ev {
                    "open" => is_open = true,
                    "close" => is_open = false,
                    "filter" => {
                        candidates_considered += 1;
                        if !is_open {
                            skipped_before_open += 1;
                            continue;
                        }
                        let confidence: FindingConfidence = to_confidence(&[], "likely");
                        let id = format!("d58/{}/{}", routine.id, cs.id);
                        let mut finding = Finding {
                            id: id.clone(),
                            root_cause_key: id,
                            detector: DETECTOR.to_string(),
                            title: "Query filter set after Open".to_string(),
                            root_cause: format!(
                                "{} sets a filter on the query variable {} AFTER Open() — \
                                 the open dataset ignores it; the filter only applies on \
                                 the next Open.",
                                routine.name, cs.callee_text
                            ),
                            severity: "medium".to_string(),
                            confidence,
                            primary_location: anchor_of(&cs.source_anchor, routine),
                            evidence_path: vec![EvidenceStep {
                                routine_id: routine.id.clone(),
                                operation_id: None,
                                callsite_id: Some(cs.id.clone()),
                                loop_id: None,
                                source_anchor: anchor_of(&cs.source_anchor, routine),
                                note: "filter after Open (ignored by the open dataset)"
                                    .to_string(),
                            }],
                            additional_paths: None,
                            affected_objects: vec![routine.object_id.clone()],
                            affected_tables: Vec::new(),
                            fix_options: vec![FixOption {
                                description: "Move the SetFilter/SetRange before Open(), or \
                                              Close() and re-Open() after changing filters."
                                    .to_string(),
                                safety: "high".to_string(),
                            }],
                            provenance: vec![Evidence {
                                source: "tree-sitter".to_string(),
                                note: None,
                            }],
                            actionable_anchor: None,
                            fingerprint: None,
                            event_kind: None,
                            cross_extension_subscribers: None,
                        };
                        finding.fingerprint = Some(fp_index.fingerprint_of(&finding));
                        findings.push(finding);
                    }
                    _ => {}
                }
            }
        }
    }

    findings.sort_by(|a, b| a.id.cmp(&b.id));
    let emitted = findings.len();
    let mut stats = DetectorStats::new(DETECTOR, candidates_considered, emitted);
    stats.add_skip("filterBeforeOpen", skipped_before_open);
    Ok(DetectorOutput::no_diag(findings, stats))
}
```

- [ ] **Step 3: Standard wiring steps.** DEFAULT lists; Smoke `detectors: &["d58-query-filter-after-open"]`.

- [ ] **Step 4: Golden expectations:** exactly **1 finding** (`FilterAfterOpen`), severity `medium`, likely. Stats: `filterBeforeOpen = 2` (`FilterBeforeOpen` + `CloseThenFilter`).

- [ ] **Step 5: Boilerplate steps 9–12.** CHANGELOG: `- d58-query-filter-after-open detector (BCQuality set-query-filters-before-open).` Commit `feat(l5): d58 query-filter-after-open detector (BCQuality wave)`.

---

### Task 11: d59 — `var Boolean` security-guard parameter on IntegrationEvent (DEFAULT)

BCQuality `integrationevent-var-parameter-bypasses-security-guards`: a writable Boolean named like a permission/skip guard on a public event lets ANY subscriber flip a security decision.

**Files:**
- Create: `src/engine/l5/detectors/d59.rs`; fixture `tests/r0-corpus/ws-d59/`; golden; standard wiring files.

**Interfaces:**
- Consumes: `routine.kind == "event-publisher"`, `has_attribute(.., "IntegrationEvent")`, `routine.parameters`.
- Produces: detector name `"d59-integrationevent-var-boolean-guard"`; helper `fn is_guard_name(&str) -> bool` (unit-tested in-module).

- [ ] **Step 1: Fixture**

`tests/r0-corpus/ws-d59/app.json` (GUID `...d590`, name `"D59 Guard Param"`).

`tests/r0-corpus/ws-d59/src/Codeunit.al`:

```al
codeunit 50933 "D59 Events"
{
    // FLAGGED: writable security-guard boolean on a public integration event.
    [IntegrationEvent(false, false)]
    procedure OnCheckAccess(UserId: Code[50]; var HasAccess: Boolean)
    begin
    end;

    // FLAGGED: skip-style guard.
    [IntegrationEvent(false, false)]
    procedure OnBeforeValidate(var SkipValidation: Boolean)
    begin
    end;

    // NOT FLAGGED: IsHandled is the sanctioned extensibility handshake.
    [IntegrationEvent(false, false)]
    procedure OnBeforePost(var IsHandled: Boolean)
    begin
    end;

    // NOT FLAGGED: non-var boolean (subscribers cannot write it).
    [IntegrationEvent(false, false)]
    procedure OnAfterCheck(HasAccess: Boolean)
    begin
    end;

    // NOT FLAGGED: var boolean without a guard-ish name.
    [IntegrationEvent(false, false)]
    procedure OnCollect(var Found: Boolean)
    begin
    end;
}
```

- [ ] **Step 2: Detector module**

`src/engine/l5/detectors/d59.rs`:

```rust
//! D59 — `var Boolean` security-guard parameter on an `[IntegrationEvent]`.
//! BCQuality `integrationevent-var-parameter-bypasses-security-guards`: any
//! subscriber (including third-party) can flip a writable Boolean that gates a
//! security decision. Name-heuristic (documented FP surface — precision-first
//! deny-list for the sanctioned IsHandled handshake).
//!
//! Severity: medium. Confidence: possible.

use crate::engine::l3::al_attributes::has_attribute;
use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::confidence::to_confidence;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::anchor_of;
use crate::engine::l5::finding::{Evidence, EvidenceStep, Finding, FindingConfidence, FixOption};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorError, DetectorOutput, DetectorStats};

const DETECTOR: &str = "d59-integrationevent-var-boolean-guard";

/// Guard-name heuristic. Deny-list first (IsHandled is the sanctioned
/// handshake), then permission/skip-shaped prefixes and substrings.
fn is_guard_name(raw: &str) -> bool {
    let n = raw.trim_matches('"').to_lowercase();
    if n == "ishandled" || n == "handled" {
        return false;
    }
    n.starts_with("skip")
        || n.starts_with("bypass")
        || n.starts_with("allow")
        || n.contains("hasaccess")
        || n.contains("permission")
        || n.contains("authoriz")
        || n.contains("authoris")
        || n == "isallowed"
        || n == "isvalid"
        || n == "cancontinue"
}

pub fn detect_d59(
    resolved: &L3Resolved,
    _ctx: &DetectorContext,
) -> Result<DetectorOutput, DetectorError> {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;
    let mut skipped_non_guard_name = 0u64;

    for routine in &ws.routines {
        if routine.kind != "event-publisher" {
            continue;
        }
        if !has_attribute(&routine.attributes_parsed, "IntegrationEvent") {
            continue;
        }
        for p in &routine.parameters {
            if !p.is_var || !p.type_text.trim().eq_ignore_ascii_case("boolean") {
                continue;
            }
            candidates_considered += 1;
            if !is_guard_name(&p.name) {
                skipped_non_guard_name += 1;
                continue;
            }

            let confidence: FindingConfidence = to_confidence(&[], "possible");
            let id = format!("d59/{}/{}", routine.id, p.index);
            let mut finding = Finding {
                id: id.clone(),
                root_cause_key: id,
                detector: DETECTOR.to_string(),
                title: "Writable security-guard parameter on integration event".to_string(),
                root_cause: format!(
                    "Integration event {} exposes `var {}: Boolean` — any subscriber \
                     (including third-party extensions) can flip this guard and bypass the \
                     security decision it feeds.",
                    routine.name, p.name
                ),
                severity: "medium".to_string(),
                confidence,
                primary_location: anchor_of(&routine.source_anchor, routine),
                evidence_path: vec![EvidenceStep {
                    routine_id: routine.id.clone(),
                    operation_id: None,
                    callsite_id: None,
                    loop_id: None,
                    source_anchor: anchor_of(&routine.source_anchor, routine),
                    note: format!("[IntegrationEvent] {} (var {}: Boolean)", routine.name, p.name),
                }],
                additional_paths: None,
                affected_objects: vec![routine.object_id.clone()],
                affected_tables: Vec::new(),
                fix_options: vec![FixOption {
                    description: format!(
                        "Make {} non-var (informational), or replace the writable guard with \
                         an explicit, audited decision API the publisher controls.",
                        p.name
                    ),
                    safety: "medium".to_string(),
                }],
                provenance: vec![Evidence {
                    source: "tree-sitter".to_string(),
                    note: None,
                }],
                actionable_anchor: None,
                fingerprint: None,
                event_kind: None,
                cross_extension_subscribers: None,
            };
            finding.fingerprint = Some(fp_index.fingerprint_of(&finding));
            findings.push(finding);
        }
    }

    findings.sort_by(|a, b| a.id.cmp(&b.id));
    let emitted = findings.len();
    let mut stats = DetectorStats::new(DETECTOR, candidates_considered, emitted);
    stats.add_skip("nonGuardName", skipped_non_guard_name);
    Ok(DetectorOutput::no_diag(findings, stats))
}

#[cfg(test)]
mod tests {
    use super::is_guard_name;

    #[test]
    fn guard_names_flagged() {
        for n in ["HasAccess", "SkipValidation", "BypassCheck", "AllowPosting", "IsAllowed",
                  "\"Has Permission\"", "Authorized"] {
            assert!(is_guard_name(n), "{n} should be a guard name");
        }
    }

    #[test]
    fn sanctioned_and_plain_names_not_flagged() {
        for n in ["IsHandled", "Handled", "Found", "Result", "Done"] {
            assert!(!is_guard_name(n), "{n} should NOT be a guard name");
        }
    }
}
```

- [ ] **Step 3: Standard wiring steps.** DEFAULT lists; Smoke `detectors: &["d59-integrationevent-var-boolean-guard"]`.

- [ ] **Step 4: Golden expectations:** exactly **2 findings** (`OnCheckAccess`/HasAccess, `OnBeforeValidate`/SkipValidation), severity `medium`, possible. Stats: `nonGuardName = 2` (`IsHandled`, `Found`); `OnAfterCheck` never a candidate (non-var).

- [ ] **Step 5: Boilerplate steps 9–12.** CHANGELOG: `- d59-integrationevent-var-boolean-guard detector (BCQuality integrationevent-var-parameter-bypasses-security-guards).` Commit `feat(l5): d59 integrationevent-var-boolean-guard detector (BCQuality wave)`.

---

### Task 12: d60 — repeat…Modify…until loop in upgrade/install codeunits (DEFAULT)

BCQuality `datatransfer-for-bulk-init`: row-by-row upgrade loops should be `DataTransfer` (set-based, no per-row triggers).

**Files:**
- Create: `src/engine/l5/detectors/d60.rs`; fixture `tests/r0-corpus/ws-d60/`; golden; standard wiring files.

**Interfaces:**
- Consumes: `L3Object.object_subtype` (`"Upgrade"`/`"Install"`), record ops `loop_stack`, `routine.loops`.
- Produces: detector name `"d60-upgrade-loop-should-be-datatransfer"`.

- [ ] **Step 1: Fixture**

`tests/r0-corpus/ws-d60/app.json` (GUID `...d600`, name `"D60 Upgrade Loop"`).

`tests/r0-corpus/ws-d60/src/Codeunit.al`:

```al
codeunit 50934 "D60 Upgrade"
{
    Subtype = Upgrade;

    // FLAGGED: row-by-row rewrite in an upgrade codeunit — DataTransfer territory.
    trigger OnUpgradePerCompany()
    var
        Item: Record "D60 Item";
    begin
        if Item.FindSet() then
            repeat
                Item.Name := 'migrated';
                Item.Modify();
            until Item.Next() = 0;
    end;
}

codeunit 50935 "D60 Normal"
{
    // NOT FLAGGED: same loop outside an upgrade/install codeunit (d5/d10 territory).
    procedure RegularLoop()
    var
        Item: Record "D60 Item";
    begin
        if Item.FindSet() then
            repeat
                Item.Name := 'x';
                Item.Modify();
            until Item.Next() = 0;
    end;
}
```

`tests/r0-corpus/ws-d60/src/Tables.al`:

```al
table 50934 "D60 Item"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; Name; Text[100]) { }
    }
    keys { key(PK; "No.") { } }
}
```

- [ ] **Step 2: Detector module**

`src/engine/l5/detectors/d60.rs`:

```rust
//! D60 — repeat…Modify…until loop in an Upgrade/Install codeunit. BCQuality
//! `datatransfer-for-bulk-init`: upgrade code rewriting rows one-by-one should
//! use DataTransfer (set-based SQL, no per-row trigger cost) — on large tables
//! the difference is hours vs seconds.
//!
//! Join: object_subtype ∈ {Upgrade, Install} (Codeunit), a Modify record op with
//! non-empty loop_stack whose receiver is a live cursor (FindSet/Find/FindFirst/
//! Next on the same var in the routine). One finding per (routine, loop, var) —
//! first op wins. Severity: medium. Confidence: likely.

use std::collections::{HashMap, HashSet};

use crate::engine::l2::features::PLoop;
use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::confidence::to_confidence;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::anchor_of;
use crate::engine::l5::finding::{Evidence, EvidenceStep, Finding, FindingConfidence, FixOption};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorError, DetectorOutput, DetectorStats};

const DETECTOR: &str = "d60-upgrade-loop-should-be-datatransfer";

const CURSOR_OPS: &[&str] = &["FindSet", "Find", "FindFirst", "Next"];

pub fn detect_d60(
    resolved: &L3Resolved,
    _ctx: &DetectorContext,
) -> Result<DetectorOutput, DetectorError> {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;
    let mut skipped_not_cursor = 0u64;

    let lifecycle_objects: HashSet<&str> = ws
        .objects
        .iter()
        .filter(|o| {
            o.object_type == "Codeunit"
                && o.object_subtype.as_deref().is_some_and(|s| {
                    s.eq_ignore_ascii_case("Upgrade") || s.eq_ignore_ascii_case("Install")
                })
        })
        .map(|o| o.id.as_str())
        .collect();
    if lifecycle_objects.is_empty() {
        let stats = DetectorStats::new(DETECTOR, 0, 0);
        return Ok(DetectorOutput::no_diag(findings, stats));
    }

    for routine in &ws.routines {
        if !lifecycle_objects.contains(routine.object_id.as_str()) {
            continue;
        }
        if !routine.body_available || routine.parse_incomplete {
            continue;
        }
        let loop_by_id: HashMap<&str, &PLoop> =
            routine.loops.iter().map(|l| (l.id.as_str(), l)).collect();
        let cursor_vars: HashSet<String> = routine
            .record_operations
            .iter()
            .filter(|op| CURSOR_OPS.contains(&op.op.as_str()))
            .map(|op| op.record_variable_name.to_lowercase())
            .collect();

        // One finding per (loop, var): first Modify wins.
        let mut reported: HashSet<(String, String)> = HashSet::new();
        for op in &routine.record_operations {
            if op.op != "Modify" {
                continue;
            }
            let Some(rep_loop_id) = op.loop_stack.last() else {
                continue;
            };
            candidates_considered += 1;
            let var_lc = op.record_variable_name.to_lowercase();
            if !cursor_vars.contains(&var_lc) {
                skipped_not_cursor += 1;
                continue;
            }
            if !reported.insert((rep_loop_id.clone(), var_lc.clone())) {
                continue;
            }
            let Some(loop_info) = loop_by_id.get(rep_loop_id.as_str()) else {
                continue;
            };

            let table_name = op
                .table_id
                .as_deref()
                .and_then(|tid| _ctx.table_by_id.get(tid).map(|t| t.name.clone()))
                .unwrap_or_else(|| op.record_variable_name.clone());

            let confidence: FindingConfidence = to_confidence(&[], "likely");
            let mut finding = Finding {
                id: format!("d60/{}/{}/{}", routine.id, loop_info.id, op.id),
                root_cause_key: format!("d60/{}/{}/{}", routine.id, loop_info.id, var_lc),
                detector: DETECTOR.to_string(),
                title: "Row-by-row upgrade loop (use DataTransfer)".to_string(),
                root_cause: format!(
                    "{} (upgrade/install codeunit) rewrites {} row-by-row in a {} loop — \
                     DataTransfer performs the same bulk init/copy set-based, without \
                     per-row trigger cost.",
                    routine.name, table_name, loop_info.loop_type
                ),
                severity: "medium".to_string(),
                confidence,
                primary_location: anchor_of(&op.source_anchor, routine),
                evidence_path: vec![
                    EvidenceStep {
                        routine_id: routine.id.clone(),
                        operation_id: None,
                        callsite_id: None,
                        loop_id: Some(loop_info.id.clone()),
                        source_anchor: anchor_of(&loop_info.source_anchor, routine),
                        note: format!("{} loop over {}", loop_info.loop_type, table_name),
                    },
                    EvidenceStep {
                        routine_id: routine.id.clone(),
                        operation_id: Some(op.id.clone()),
                        callsite_id: None,
                        loop_id: Some(loop_info.id.clone()),
                        source_anchor: anchor_of(&op.source_anchor, routine),
                        note: "per-row Modify".to_string(),
                    },
                ],
                additional_paths: None,
                affected_objects: vec![routine.object_id.clone()],
                affected_tables: op.table_id.iter().cloned().collect(),
                fix_options: vec![FixOption {
                    description: "Replace the loop with a DataTransfer (SourceTable/\
                                  DestinationTable + CopyFields/ConstantValue), or ModifyAll \
                                  when a single field gets a constant."
                        .to_string(),
                    safety: "medium".to_string(),
                }],
                provenance: vec![Evidence {
                    source: "tree-sitter".to_string(),
                    note: None,
                }],
                actionable_anchor: None,
                fingerprint: None,
                event_kind: None,
                cross_extension_subscribers: None,
            };
            finding.fingerprint = Some(fp_index.fingerprint_of(&finding));
            findings.push(finding);
        }
    }

    findings.sort_by(|a, b| a.id.cmp(&b.id));
    let emitted = findings.len();
    let mut stats = DetectorStats::new(DETECTOR, candidates_considered, emitted);
    stats.add_skip("notCursorVar", skipped_not_cursor);
    Ok(DetectorOutput::no_diag(findings, stats))
}
```

(Rename `_ctx` to `ctx` — it IS used for `table_by_id`. Keep the parameter name `ctx`.)

- [ ] **Step 3: Standard wiring steps.** DEFAULT lists; Smoke `detectors: &["d60-upgrade-loop-should-be-datatransfer"]`.

- [ ] **Step 4: Golden expectations:** exactly **1 finding** (`OnUpgradePerCompany` in `"D60 Upgrade"`), severity `medium`, likely. `"D60 Normal"` produces nothing (not a lifecycle object).

- [ ] **Step 5: Boilerplate steps 9–12.** CHANGELOG: `- d60-upgrade-loop-should-be-datatransfer detector (BCQuality datatransfer-for-bulk-init).` Commit `feat(l5): d60 upgrade-loop-should-be-datatransfer detector (BCQuality wave)`.

---

### Task 13: d61 — IsHandled bypasses critical writes (OPT-IN)

BCQuality `do-not-bypass-critical-operations-with-ishandled`. OPT-IN because d43 (event-ishandled-skip) already covers the generic IsHandled-skip shape — d61 refines it to "what gets skipped is a WRITE", for deep audits.

**Files:**
- Create: `src/engine/l5/detectors/d61.rs`; fixture `tests/r0-corpus/ws-d61/`; golden; standard wiring files (OPT-IN placement: registry after d51, `OPT_IN_DETECTOR_NAMES`).

**Interfaces:**
- Consumes: `ctx.event_graph` (events + edges), `ctx.resolved_call_edge_by_callsite`, `routine.condition_references`, `routine.var_assignments`, `routine.record_operations`, `before_anchor`.
- Produces: detector name `"d61-ishandled-bypasses-critical-write"`; reuses Task 8's `anchor_within` shape (duplicate the 10-line local helper — detectors keep local geometry helpers local; promote to mod.rs only if a third consumer appears).

- [ ] **Step 1: Fixture**

`tests/r0-corpus/ws-d61/app.json` (GUID `...d610`, name `"D61 IsHandled Write"`).

`tests/r0-corpus/ws-d61/src/Codeunit.al`:

```al
codeunit 50936 "D61 Events"
{
    [IntegrationEvent(false, false)]
    procedure OnBeforePost(var IsHandled: Boolean)
    begin
    end;

    [IntegrationEvent(false, false)]
    procedure OnBeforeLog(var IsHandled: Boolean)
    begin
    end;
}

codeunit 50937 "D61 Poster"
{
    // The guarded critical write: subscriber flipping IsHandled skips the Modify.
    procedure Post()
    var
        Ev: Codeunit "D61 Events";
        Item: Record "D61 Item";
        IsHandled: Boolean;
    begin
        Ev.OnBeforePost(IsHandled);
        if not IsHandled then begin
            Item.FindFirst();
            Item.Posted := true;
            Item.Modify();
        end;
    end;

    // Guard skips only a Message — nothing critical; not flagged.
    procedure Log()
    var
        Ev: Codeunit "D61 Events";
        IsHandled: Boolean;
    begin
        Ev.OnBeforeLog(IsHandled);
        if not IsHandled then
            Message('logged');
    end;
}

codeunit 50938 "D61 Subscribers"
{
    // FLAGGED (with the Post guard): unconditionally claims handled — the
    // publisher-side Modify is silently skipped.
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"D61 Events", 'OnBeforePost', '', false, false)]
    local procedure HandlePost(var IsHandled: Boolean)
    begin
        IsHandled := true;
    end;

    // NOT FLAGGED: subscribes to the Message-only event.
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"D61 Events", 'OnBeforeLog', '', false, false)]
    local procedure HandleLog(var IsHandled: Boolean)
    begin
        IsHandled := true;
    end;
}
```

`tests/r0-corpus/ws-d61/src/Tables.al`:

```al
table 50936 "D61 Item"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; Posted; Boolean) { }
    }
    keys { key(PK; "No.") { } }
}
```

- [ ] **Step 2: Detector module**

`src/engine/l5/detectors/d61.rs`:

```rust
//! D61 — `IsHandled := true` bypasses a critical write (OPT-IN). BCQuality
//! `do-not-bypass-critical-operations-with-ishandled`. d43 flags the generic
//! IsHandled-skip shape; d61 refines: the publisher-side guard protects a
//! record WRITE, and a subscriber provably sets the flag — the write is
//! silently skippable by an extension.
//!
//! Join (every leg exact, every uncertainty skips):
//!  1. publisher: event-publisher routine with a var Boolean param named
//!     ishandled/handled;
//!  2. caller: routine with a RESOLVED call to that publisher, binding a local
//!     var to the IsHandled param; a post-call `if` guard on that var
//!     (condition_references) whose statement contains a record write op;
//!  3. subscriber: an event-graph subscriber of the same event assigning
//!     literal `true` to its own ishandled/handled param.
//!
//! Finding per (caller callsite × subscriber). Severity: high. Confidence:
//! likely when the subscriber body has NO branching (unconditional claim),
//! else possible. Inert on the cross-app context (resolver join empty).

use std::collections::HashMap;

use crate::engine::l2::features::PAnchor;
use crate::engine::l3::l3_workspace::{L3Resolved, L3Routine};
use crate::engine::l5::confidence::to_confidence;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::{anchor_of, before_anchor};
use crate::engine::l5::finding::{Evidence, EvidenceStep, Finding, FindingConfidence, FixOption};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorError, DetectorOutput, DetectorStats};

const DETECTOR: &str = "d61-ishandled-bypasses-critical-write";

const CRITICAL_WRITE_OPS: &[&str] =
    &["Insert", "Modify", "Delete", "DeleteAll", "ModifyAll", "Rename"];

fn is_ishandled_name(raw: &str) -> bool {
    let n = raw.trim_matches('"').to_lowercase();
    n == "ishandled" || n == "handled"
}

fn anchor_within(inner: &PAnchor, outer: &PAnchor) -> bool {
    let starts_ok = outer.start_line < inner.start_line
        || (outer.start_line == inner.start_line && outer.start_column <= inner.start_column);
    let ends_ok = inner.end_line < outer.end_line
        || (inner.end_line == outer.end_line && inner.end_column <= outer.end_column);
    starts_ok && ends_ok
}

pub fn detect_d61(
    resolved: &L3Resolved,
    ctx: &DetectorContext,
) -> Result<DetectorOutput, DetectorError> {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;
    let mut skipped_no_critical_write = 0u64;
    let mut skipped_no_flipping_subscriber = 0u64;

    // Leg 1: IsHandled-pattern publishers → (param index, event id).
    let mut publisher_meta: HashMap<&str, (u32, &str)> = HashMap::new();
    let mut event_by_publisher: HashMap<&str, &str> = HashMap::new();
    for ev in &ctx.event_graph.events {
        if let Some(pr) = &ev.publisher_routine_id {
            event_by_publisher.insert(pr.as_str(), ev.id.as_str());
        }
    }
    for r in &ws.routines {
        if r.kind != "event-publisher" {
            continue;
        }
        let Some(p) = r
            .parameters
            .iter()
            .find(|p| p.is_var && p.type_text.trim().eq_ignore_ascii_case("boolean")
                && is_ishandled_name(&p.name))
        else {
            continue;
        };
        let Some(&event_id) = event_by_publisher.get(r.id.as_str()) else {
            continue;
        };
        publisher_meta.insert(r.id.as_str(), (p.index, event_id));
    }
    if publisher_meta.is_empty() {
        let stats = DetectorStats::new(DETECTOR, 0, 0);
        return Ok(DetectorOutput::no_diag(findings, stats));
    }

    // Leg 3: subscribers that assign literal true to their ishandled param,
    // keyed by event id.
    let routine_by_id = &ctx.routine_by_id;
    let mut flippers_by_event: HashMap<&str, Vec<(&L3Routine, &PAnchor)>> = HashMap::new();
    for edge in &ctx.event_graph.edges {
        let Some(sub) = routine_by_id.get(edge.subscriber_routine_id.as_str()) else {
            continue;
        };
        if !sub.body_available || sub.parse_incomplete {
            continue;
        }
        let Some(asg) = sub.var_assignments.iter().find(|a| {
            is_ishandled_name(&a.lhs_name)
                && a.rhs_literal_value.as_deref().is_some_and(|v| v.eq_ignore_ascii_case("true"))
        }) else {
            continue;
        };
        flippers_by_event
            .entry(edge.event_id.as_str())
            .or_default()
            .push((sub, &asg.source_anchor));
    }

    // Leg 2: callers with a post-call guard protecting a critical write.
    for caller in &ws.routines {
        if !caller.body_available || caller.parse_incomplete {
            continue;
        }
        for cs in &caller.call_sites {
            let Some(edge) = ctx.resolved_call_edge_by_callsite.get(&cs.id) else {
                continue;
            };
            let Some(to) = edge.to.as_deref() else {
                continue;
            };
            let Some(&(param_index, event_id)) = publisher_meta.get(to) else {
                continue;
            };
            let Some(publisher) = routine_by_id.get(to) else {
                continue;
            };
            // The caller-side variable bound to the IsHandled param.
            let Some(guard_var) = cs
                .argument_bindings
                .iter()
                .find(|b| b.parameter_index == param_index)
                .and_then(|b| b.source_variable_name.as_deref())
            else {
                continue;
            };
            // Post-call guard statement referencing that var.
            let Some(guard) = caller.condition_references.iter().find(|cr| {
                cr.identifier.to_lowercase() == guard_var
                    && before_anchor(&cs.source_anchor, &cr.statement_anchor)
            }) else {
                continue;
            };
            candidates_considered += 1;
            // A critical write inside the guarded statement.
            let Some(write) = caller.record_operations.iter().find(|op| {
                CRITICAL_WRITE_OPS.contains(&op.op.as_str())
                    && anchor_within(&op.source_anchor, &guard.statement_anchor)
            }) else {
                skipped_no_critical_write += 1;
                continue;
            };
            let Some(flippers) = flippers_by_event.get(event_id) else {
                skipped_no_flipping_subscriber += 1;
                continue;
            };

            for (sub, asg_anchor) in flippers {
                let unconditional = !sub.has_branching;
                let confidence: FindingConfidence =
                    to_confidence(&[], if unconditional { "likely" } else { "possible" });
                let mut finding = Finding {
                    id: format!("d61/{}/{}/{}", caller.id, cs.id, sub.id),
                    root_cause_key: format!("d61/{}/{}", caller.id, cs.id),
                    detector: DETECTOR.to_string(),
                    title: "IsHandled bypasses critical write".to_string(),
                    root_cause: format!(
                        "{} guards a {} on {} behind `if not {}` after publishing {}; \
                         subscriber {} sets {} := true{} — the write is silently skipped.",
                        caller.name,
                        write.op,
                        write.record_variable_name,
                        guard_var,
                        publisher.name,
                        sub.name,
                        guard_var,
                        if unconditional { " unconditionally" } else { "" }
                    ),
                    severity: "high".to_string(),
                    confidence,
                    primary_location: anchor_of(&write.source_anchor, caller),
                    evidence_path: vec![
                        EvidenceStep {
                            routine_id: caller.id.clone(),
                            operation_id: None,
                            callsite_id: Some(cs.id.clone()),
                            loop_id: None,
                            source_anchor: anchor_of(&cs.source_anchor, caller),
                            note: format!("publishes {}", publisher.name),
                        },
                        EvidenceStep {
                            routine_id: caller.id.clone(),
                            operation_id: Some(write.id.clone()),
                            callsite_id: None,
                            loop_id: None,
                            source_anchor: anchor_of(&write.source_anchor, caller),
                            note: format!("guarded critical {}", write.op),
                        },
                        EvidenceStep {
                            routine_id: sub.id.clone(),
                            operation_id: None,
                            callsite_id: None,
                            loop_id: None,
                            source_anchor: anchor_of(asg_anchor, sub),
                            note: format!("subscriber sets {guard_var} := true"),
                        },
                    ],
                    additional_paths: None,
                    affected_objects: vec![caller.object_id.clone(), sub.object_id.clone()],
                    affected_tables: write.table_id.iter().cloned().collect(),
                    fix_options: vec![FixOption {
                        description: "If the subscriber replaces the write, make it perform an \
                                      equivalent durable operation; otherwise restrict the \
                                      IsHandled contract to non-critical steps (split the event)."
                            .to_string(),
                        safety: "low".to_string(),
                    }],
                    provenance: vec![Evidence {
                        source: "tree-sitter".to_string(),
                        note: None,
                    }],
                    actionable_anchor: None,
                    fingerprint: None,
                    event_kind: None,
                    cross_extension_subscribers: None,
                };
                finding.fingerprint = Some(fp_index.fingerprint_of(&finding));
                findings.push(finding);
            }
        }
    }

    findings.sort_by(|a, b| a.id.cmp(&b.id));
    let emitted = findings.len();
    let mut stats = DetectorStats::new(DETECTOR, candidates_considered, emitted);
    stats.add_skip("noCriticalWrite", skipped_no_critical_write);
    stats.add_skip("noFlippingSubscriber", skipped_no_flipping_subscriber);
    Ok(DetectorOutput::no_diag(findings, stats))
}
```

- [ ] **Step 3: Standard wiring steps** — OPT-IN placement: registry entry AFTER d51, name in `OPT_IN_DETECTOR_NAMES`. Smoke `detectors: &["d61-ishandled-bypasses-critical-write"]` (run_smoke_entry selects by name, so an opt-in detector still runs for its fixture).

- [ ] **Step 4: Golden expectations:** exactly **1 finding** (`Post` × `HandlePost`), severity `high`, confidence `likely` (HandlePost has no branching). Stats: `noCriticalWrite = 1` (the `Log` guard). If `condition_references.identifier` turns out to be stored lowercased already, the `.to_lowercase()` is harmless; if the guard statement anchor does not span the `begin..end` block, the fixture will show 0 findings at regen — STOP, verify the real `statement_anchor` extent (d43's usage is the reference), fix the containment logic, re-regen. Do not weaken the fixture to pass.

- [ ] **Step 5: Boilerplate steps 9–12.** CHANGELOG: `- d61-ishandled-bypasses-critical-write detector (opt-in; BCQuality do-not-bypass-critical-operations-with-ishandled).` Commit `feat(l5): d61 ishandled-bypasses-critical-write detector (BCQuality wave, opt-in)`.

---

### Task 14: d62 — FeatureTelemetry.LogUsage before the success path (OPT-IN)

BCQuality `feature-usage-only-after-success`: usage telemetry logged before the operation can still fail overcounts the feature.

**Files:**
- Create: `src/engine/l5/detectors/d62.rs`; fixture `tests/r0-corpus/ws-d62/`; golden; standard wiring files (OPT-IN placement).

**Interfaces:**
- Consumes: `routine.variables` (declared type contains `Feature Telemetry`), member call sites, record ops, `operation_sites` (`kind == "error-call"`), `before_anchor`.
- Produces: detector name `"d62-telemetry-before-success"`.

- [ ] **Step 1: Fixture**

`tests/r0-corpus/ws-d62/app.json` (GUID `...d620`, name `"D62 Telemetry Order"`).

`tests/r0-corpus/ws-d62/src/Codeunit.al`:

```al
codeunit 50939 "D62 Demo"
{
    // FLAGGED: LogUsage before the fallible write — failure after the log
    // overcounts the feature.
    procedure LogThenWork()
    var
        FeatureTelemetry: Codeunit "Feature Telemetry";
        Item: Record "D62 Item";
    begin
        FeatureTelemetry.LogUsage('D62', 'Demo', 'started');
        Item.Init();
        Item.Insert();
    end;

    // NOT FLAGGED: log after the last fallible operation.
    procedure WorkThenLog()
    var
        FeatureTelemetry: Codeunit "Feature Telemetry";
        Item: Record "D62 Item";
    begin
        Item.Init();
        Item.Insert();
        FeatureTelemetry.LogUsage('D62', 'Demo', 'done');
    end;
}
```

`tests/r0-corpus/ws-d62/src/Tables.al`:

```al
table 50939 "D62 Item"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }
    keys { key(PK; "No.") { } }
}
```

(The fixture has no real "Feature Telemetry" codeunit — the detector matches the DECLARED TYPE TEXT, no resolution needed, mirroring how BC apps reference the System Application codeunit.)

- [ ] **Step 2: Detector module**

`src/engine/l5/detectors/d62.rs`:

```rust
//! D62 — `FeatureTelemetry.LogUsage` before the success path (OPT-IN).
//! BCQuality `feature-usage-only-after-success`: usage logged before a fallible
//! step (record write or explicit Error call later in the routine) counts
//! failed runs as feature usage.
//!
//! Join: member call `<v>.LogUsage(..)` where `<v>`'s DECLARED type contains
//! `codeunit "feature telemetry"` (text match — the System Application codeunit
//! is not in workspace source), with any record write op or error-call
//! operation site strictly AFTER it in the same routine (straight-line source
//! order). Severity: low. Confidence: possible.

use crate::engine::l2::features::PCallee;
use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::confidence::to_confidence;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::{anchor_of, before_anchor};
use crate::engine::l5::finding::{Evidence, EvidenceStep, Finding, FindingConfidence, FixOption};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorError, DetectorOutput, DetectorStats};

const DETECTOR: &str = "d62-telemetry-before-success";

const WRITE_OPS: &[&str] = &["Insert", "Modify", "Delete", "DeleteAll", "ModifyAll", "Rename"];

pub fn detect_d62(
    resolved: &L3Resolved,
    _ctx: &DetectorContext,
) -> Result<DetectorOutput, DetectorError> {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;
    let mut skipped_terminal_log = 0u64;

    for routine in &ws.routines {
        if !routine.body_available || routine.parse_incomplete {
            continue;
        }
        let ft_vars: Vec<String> = routine
            .variables
            .iter()
            .filter(|v| {
                let t = v.declared_type.to_lowercase();
                t.starts_with("codeunit") && t.contains("feature telemetry")
            })
            .map(|v| v.name.to_lowercase())
            .collect();
        if ft_vars.is_empty() {
            continue;
        }

        for cs in &routine.call_sites {
            let PCallee::Member { receiver, method } = &cs.callee else {
                continue;
            };
            if !method.eq_ignore_ascii_case("LogUsage") {
                continue;
            }
            if !ft_vars.contains(&receiver.to_lowercase()) {
                continue;
            }
            candidates_considered += 1;

            let fallible_after = routine.record_operations.iter().any(|op| {
                WRITE_OPS.contains(&op.op.as_str())
                    && before_anchor(&cs.source_anchor, &op.source_anchor)
            }) || routine.operation_sites.iter().any(|s| {
                s.kind == "error-call" && before_anchor(&cs.source_anchor, &s.source_anchor)
            });
            if !fallible_after {
                skipped_terminal_log += 1;
                continue;
            }

            let confidence: FindingConfidence = to_confidence(&[], "possible");
            let id = format!("d62/{}/{}", routine.id, cs.id);
            let mut finding = Finding {
                id: id.clone(),
                root_cause_key: id,
                detector: DETECTOR.to_string(),
                title: "Feature usage logged before success".to_string(),
                root_cause: format!(
                    "{} calls FeatureTelemetry.LogUsage before fallible work later in the \
                     routine — runs that fail after the log still count as feature usage.",
                    routine.name
                ),
                severity: "low".to_string(),
                confidence,
                primary_location: anchor_of(&cs.source_anchor, routine),
                evidence_path: vec![EvidenceStep {
                    routine_id: routine.id.clone(),
                    operation_id: None,
                    callsite_id: Some(cs.id.clone()),
                    loop_id: None,
                    source_anchor: anchor_of(&cs.source_anchor, routine),
                    note: "LogUsage before fallible operations".to_string(),
                }],
                additional_paths: None,
                affected_objects: vec![routine.object_id.clone()],
                affected_tables: Vec::new(),
                fix_options: vec![FixOption {
                    description: "Move LogUsage after the operation's success point (end of \
                                  the routine / after the final write)."
                        .to_string(),
                    safety: "high".to_string(),
                }],
                provenance: vec![Evidence {
                    source: "tree-sitter".to_string(),
                    note: None,
                }],
                actionable_anchor: None,
                fingerprint: None,
                event_kind: None,
                cross_extension_subscribers: None,
            };
            finding.fingerprint = Some(fp_index.fingerprint_of(&finding));
            findings.push(finding);
        }
    }

    findings.sort_by(|a, b| a.id.cmp(&b.id));
    let emitted = findings.len();
    let mut stats = DetectorStats::new(DETECTOR, candidates_considered, emitted);
    stats.add_skip("terminalLog", skipped_terminal_log);
    Ok(DetectorOutput::no_diag(findings, stats))
}
```

- [ ] **Step 3: Standard wiring steps** — OPT-IN placement.
- [ ] **Step 4: Golden expectations:** exactly **1 finding** (`LogThenWork`), severity `low`, possible; stats `terminalLog = 1` (`WorkThenLog`).
- [ ] **Step 5: Boilerplate steps 9–12.** CHANGELOG: `- d62-telemetry-before-success detector (opt-in; BCQuality feature-usage-only-after-success).` Commit `feat(l5): d62 telemetry-before-success detector (BCQuality wave, opt-in)`.

---

### Task 15: d63 — HTML built by string concatenation (OPT-IN heuristic)

BCQuality `al-has-no-built-in-htmlencode`: AL has no HtmlEncode; concatenating data into HTML literals is an XSS-shaped injection risk. Text heuristic over call-site argument texts — deliberately OPT-IN.

**Files:**
- Create: `src/engine/l5/detectors/d63.rs`; fixture `tests/r0-corpus/ws-d63/`; golden; standard wiring files (OPT-IN placement).

**Interfaces:**
- Consumes: `cs.argument_texts` (raw argument source text).
- Produces: detector name `"d63-html-concat-injection"`; helpers `looks_like_html_concat(&str) -> bool`, `html_tagish(&str) -> bool` (unit-tested in-module — the real TDD surface of this task).

- [ ] **Step 1: Write the failing unit tests FIRST** (in the new module, before the detector body is finished):

```rust
#[cfg(test)]
mod tests {
    use super::{html_tagish, looks_like_html_concat};

    #[test]
    fn html_literal_plus_concat_flags() {
        assert!(looks_like_html_concat("'<b>' + UserName + '</b>'"));
        assert!(looks_like_html_concat("'<div class=x>' + V"));
        assert!(looks_like_html_concat("Body + '</table>'"));
    }

    #[test]
    fn plain_literals_and_math_do_not_flag() {
        assert!(!looks_like_html_concat("'<b>static</b>'")); // no concat
        assert!(!looks_like_html_concat("'a' + 'b'")); // concat, no HTML tag... see note
        assert!(!looks_like_html_concat("X + Y")); // no literal
        assert!(!looks_like_html_concat("'2 < 3 and 4 > 1' + V")); // `< ` not tag-ish
    }

    #[test]
    fn escaped_quotes_inside_literals_handled() {
        assert!(!looks_like_html_concat("'it''s fine' + V"));
        assert!(looks_like_html_concat("'it''s <b>' + V"));
    }

    #[test]
    fn tagish_needs_letter_or_slash_after_lt() {
        assert!(html_tagish("<b>"));
        assert!(html_tagish("</td>"));
        assert!(!html_tagish("2 < 3"));
        assert!(!html_tagish("no tags"));
    }
}
```

- [ ] **Step 2: Fixture**

`tests/r0-corpus/ws-d63/app.json` (GUID `...d630`, name `"D63 Html Concat"`).

`tests/r0-corpus/ws-d63/src/Codeunit.al`:

```al
codeunit 50940 "D63 Demo"
{
    // FLAGGED: HTML literal concatenated with data — no HtmlEncode in AL.
    procedure BuildUnsafe(UserName: Text): Text
    begin
        exit(Render('<b>' + UserName + '</b>'));
    end;

    // NOT FLAGGED: static HTML, no concatenation.
    procedure BuildStatic(): Text
    begin
        exit(Render('<b>static</b>'));
    end;

    // NOT FLAGGED: concatenation without HTML literals.
    procedure BuildPlain(UserName: Text): Text
    begin
        exit(Render('Hello ' + UserName));
    end;

    local procedure Render(Html: Text): Text
    begin
        exit(Html);
    end;
}
```

- [ ] **Step 3: Detector module**

`src/engine/l5/detectors/d63.rs`:

```rust
//! D63 — HTML built by string concatenation (OPT-IN heuristic). BCQuality
//! `al-has-no-built-in-htmlencode`: AL has no HtmlEncode; splicing data into
//! HTML literals is an injection (XSS-shaped) risk wherever the string reaches
//! a browser/mail surface. Pure TEXT heuristic over call-site argument source
//! text — one finding per call site (first matching argument), OPT-IN because
//! the engine cannot see where the string ends up.
//!
//! Severity: low. Confidence: possible.

use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::confidence::to_confidence;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::detectors::anchor_of;
use crate::engine::l5::finding::{Evidence, EvidenceStep, Finding, FindingConfidence, FixOption};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorError, DetectorOutput, DetectorStats};

const DETECTOR: &str = "d63-html-concat-injection";

/// Does the literal contain an HTML-tag-ish `<x` / `</x` sequence?
fn html_tagish(lit: &str) -> bool {
    let b = lit.as_bytes();
    b.windows(2)
        .any(|w| w[0] == b'<' && (w[1].is_ascii_alphabetic() || w[1] == b'/'))
}

/// Argument-text heuristic: at least one single-quoted AL literal containing an
/// HTML-tag-ish sequence AND at least one `+` OUTSIDE the literals (a real
/// concatenation). AL escapes `'` inside literals as `''`.
fn looks_like_html_concat(text: &str) -> bool {
    let mut in_lit = false;
    let mut lit = String::new();
    let mut html_lit = false;
    let mut concat_outside = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_lit {
            if c == '\'' {
                if chars.peek() == Some(&'\'') {
                    chars.next();
                    lit.push('\'');
                } else {
                    in_lit = false;
                    if html_tagish(&lit) {
                        html_lit = true;
                    }
                    lit.clear();
                }
            } else {
                lit.push(c);
            }
        } else if c == '\'' {
            in_lit = true;
        } else if c == '+' {
            concat_outside = true;
        }
    }
    html_lit && concat_outside
}

pub fn detect_d63(
    resolved: &L3Resolved,
    _ctx: &DetectorContext,
) -> Result<DetectorOutput, DetectorError> {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;

    for routine in &ws.routines {
        if !routine.body_available || routine.parse_incomplete {
            continue;
        }
        for cs in &routine.call_sites {
            let Some(arg) = cs.argument_texts.iter().find(|t| looks_like_html_concat(t))
            else {
                continue;
            };
            candidates_considered += 1;

            let confidence: FindingConfidence = to_confidence(&[], "possible");
            let id = format!("d63/{}/{}", routine.id, cs.id);
            let mut finding = Finding {
                id: id.clone(),
                root_cause_key: id,
                detector: DETECTOR.to_string(),
                title: "HTML built by string concatenation".to_string(),
                root_cause: format!(
                    "{} concatenates data into an HTML literal ({}) — AL has no built-in \
                     HtmlEncode, so any user-influenced value is an injection risk where \
                     this string reaches a browser or mail body.",
                    routine.name,
                    arg.chars().take(60).collect::<String>()
                ),
                severity: "low".to_string(),
                confidence,
                primary_location: anchor_of(&cs.source_anchor, routine),
                evidence_path: vec![EvidenceStep {
                    routine_id: routine.id.clone(),
                    operation_id: None,
                    callsite_id: Some(cs.id.clone()),
                    loop_id: None,
                    source_anchor: anchor_of(&cs.source_anchor, routine),
                    note: "HTML literal + concatenation in argument".to_string(),
                }],
                additional_paths: None,
                affected_objects: vec![routine.object_id.clone()],
                affected_tables: Vec::new(),
                fix_options: vec![FixOption {
                    description: "Encode interpolated values (replace <, >, &, \" before \
                                  splicing) or build the document with an XmlDocument/\
                                  template API instead of concatenation."
                        .to_string(),
                    safety: "medium".to_string(),
                }],
                provenance: vec![Evidence {
                    source: "tree-sitter".to_string(),
                    note: None,
                }],
                actionable_anchor: None,
                fingerprint: None,
                event_kind: None,
                cross_extension_subscribers: None,
            };
            finding.fingerprint = Some(fp_index.fingerprint_of(&finding));
            findings.push(finding);
        }
    }

    findings.sort_by(|a, b| a.id.cmp(&b.id));
    let emitted = findings.len();
    let stats = DetectorStats::new(DETECTOR, candidates_considered, emitted);
    Ok(DetectorOutput::no_diag(findings, stats))
}
```

(Plus the `#[cfg(test)] mod tests` from Step 1 at the bottom of the module. Note: `'a' + 'b'` HAS a concat but no HTML tag → not flagged; the unit test comment documents this is tag-driven, not concat-driven.)

- [ ] **Step 4: Run unit tests**: `cargo test -p al_call_hierarchy --lib d63 -- --nocapture` → PASS.
- [ ] **Step 5: Standard wiring steps** — OPT-IN placement.
- [ ] **Step 6: Golden expectations:** exactly **1 finding** (`BuildUnsafe`), severity `low`, possible. `BuildStatic`/`BuildPlain` never candidates.
- [ ] **Step 7: Boilerplate steps 9–12.** CHANGELOG: `- d63-html-concat-injection detector (opt-in heuristic; BCQuality al-has-no-built-in-htmlencode).` Commit `feat(l5): d63 html-concat-injection detector (BCQuality wave, opt-in)`.

---

### Task 16: d64 — API page write-surface exposure (OPT-IN; needs Task 2)

BCQuality `disable-write-operations-on-read-only-api-pages` (+ the explicit-write-surface half of `expose-only-committed-data-from-api-reads` — the ReadIsolation body-signal is NOT detectable from the current model and is explicitly out of scope; record that in the module doc).

**Files:**
- Create: `src/engine/l5/detectors/d64.rs`; fixture `tests/r0-corpus/ws-d64/`; golden; standard wiring files (OPT-IN placement).

**Interfaces:**
- Consumes: `L3Object.{page_type, editable, insert_allowed, modify_allowed, delete_allowed, source_anchor}` (Task 2).
- Produces: detector name `"d64-api-page-write-surface"`. Object-level findings: `EvidenceStep.routine_id` carries the OBJECT id (no routine exists on a declarative page — documented convention).

- [ ] **Step 1: Fixture**

`tests/r0-corpus/ws-d64/app.json` (GUID `...d640`, name `"D64 Api Surface"`).

`tests/r0-corpus/ws-d64/src/Pages.al`:

```al
table 50941 "D64 Item"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }
    keys { key(PK; "No.") { } }
}

// FLAGGED (shape A, low): declared read-only but write operations not disabled.
page 50941 "D64 ReadOnly Leaky"
{
    PageType = API;
    SourceTable = "D64 Item";
    Editable = false;

    layout
    {
        area(Content)
        {
            field(No; Rec."No.") { }
        }
    }
}

// FLAGGED (shape B, info): no explicit write-surface declaration at all.
page 50942 "D64 Undeclared"
{
    PageType = API;
    SourceTable = "D64 Item";

    layout
    {
        area(Content)
        {
            field(No; Rec."No.") { }
        }
    }
}

// NOT FLAGGED: write surface explicitly closed.
page 50943 "D64 Closed"
{
    PageType = API;
    SourceTable = "D64 Item";
    Editable = false;
    InsertAllowed = false;
    ModifyAllowed = false;
    DeleteAllowed = false;

    layout
    {
        area(Content)
        {
            field(No; Rec."No.") { }
        }
    }
}

// NOT FLAGGED: not an API page.
page 50944 "D64 Card"
{
    PageType = Card;
    SourceTable = "D64 Item";

    layout
    {
        area(Content)
        {
            field(No; Rec."No.") { }
        }
    }
}
```

- [ ] **Step 2: Detector module**

`src/engine/l5/detectors/d64.rs`:

```rust
//! D64 — API page write-surface exposure (OPT-IN). BCQuality
//! `disable-write-operations-on-read-only-api-pages`: an API page that is
//! Editable=false but leaves Insert/Modify/DeleteAllowed unset still exposes
//! OData writes (shape A, low). An API page declaring NO write-surface
//! property at all ships the default-open surface silently (shape B, info).
//!
//! OUT OF SCOPE (recorded here deliberately): the ReadIsolation :=
//! ReadCommitted body signal from `expose-only-committed-data-from-api-reads`
//! — member-property assignments are not captured by the L2 walk; revisit if
//! identifier_references ever carry member writes.
//!
//! Object-level findings: the page may have NO routines, so the evidence
//! step's routine_id carries the OBJECT id and the anchor is the object's own
//! decl anchor (Task-2 `L3Object.source_anchor`), falling back to a 1:1 anchor
//! in the object's first source unit when absent.

use crate::engine::l3::l3_workspace::L3Resolved;
use crate::engine::l5::confidence::to_confidence;
use crate::engine::l5::detector_context::DetectorContext;
use crate::engine::l5::finding::{
    Evidence, EvidenceStep, Finding, FindingConfidence, FixOption, SourceAnchor,
};
use crate::engine::l5::fingerprint::FingerprintIndex;
use crate::engine::l5::registry::{DetectorError, DetectorOutput, DetectorStats};

const DETECTOR: &str = "d64-api-page-write-surface";

fn object_anchor(o: &crate::engine::l3::l3_workspace::L3Object) -> SourceAnchor {
    match &o.source_anchor {
        Some(a) => SourceAnchor {
            source_unit_id: a.source_unit_id.clone(),
            start_line: a.start_line,
            start_column: a.start_column,
            end_line: a.end_line,
            end_column: a.end_column,
            enclosing_routine_id: o.id.clone(), // object-level: object id by convention
            syntax_kind: a.syntax_kind.clone(),
            normalized_text_hash: None,
            leading_context_hash: None,
            trailing_context_hash: None,
        },
        None => SourceAnchor {
            source_unit_id: String::new(),
            start_line: 1,
            start_column: 1,
            end_line: 1,
            end_column: 1,
            enclosing_routine_id: o.id.clone(),
            syntax_kind: "object".to_string(),
            normalized_text_hash: None,
            leading_context_hash: None,
            trailing_context_hash: None,
        },
    }
}

pub fn detect_d64(
    resolved: &L3Resolved,
    _ctx: &DetectorContext,
) -> Result<DetectorOutput, DetectorError> {
    let ws = &resolved.workspace;
    let fp_index = FingerprintIndex::build(&ws.routines, &ws.objects);
    let mut findings: Vec<Finding> = Vec::new();
    let mut candidates_considered = 0usize;
    let mut skipped_declared_closed = 0u64;

    for o in &ws.objects {
        if o.object_type != "Page" {
            continue;
        }
        if !o.page_type.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("api")) {
            continue;
        }
        candidates_considered += 1;

        let writes_closed = o.insert_allowed == Some(false)
            && o.modify_allowed == Some(false)
            && o.delete_allowed == Some(false);
        let nothing_declared = o.editable.is_none()
            && o.insert_allowed.is_none()
            && o.modify_allowed.is_none()
            && o.delete_allowed.is_none();

        let (severity, title, root_cause) = if o.editable == Some(false) && !writes_closed {
            let mut missing: Vec<&str> = Vec::new();
            if o.insert_allowed != Some(false) {
                missing.push("InsertAllowed");
            }
            if o.modify_allowed != Some(false) {
                missing.push("ModifyAllowed");
            }
            if o.delete_allowed != Some(false) {
                missing.push("DeleteAllowed");
            }
            (
                "low",
                "Read-only API page leaves write operations enabled".to_string(),
                format!(
                    "API page {} declares Editable = false but does not disable {} — the \
                     OData surface still accepts those writes.",
                    o.name,
                    missing.join("/")
                ),
            )
        } else if nothing_declared {
            (
                "info",
                "API page write surface not declared".to_string(),
                format!(
                    "API page {} declares none of Editable/InsertAllowed/ModifyAllowed/\
                     DeleteAllowed — the default-open write surface ships silently; \
                     declare the intent explicitly.",
                    o.name
                ),
            )
        } else {
            skipped_declared_closed += 1;
            continue;
        };

        let anchor = object_anchor(o);
        let confidence: FindingConfidence = to_confidence(&[], "possible");
        let id = format!("d64/{}", o.id);
        let mut finding = Finding {
            id: id.clone(),
            root_cause_key: id,
            detector: DETECTOR.to_string(),
            title,
            root_cause,
            severity: severity.to_string(),
            confidence,
            primary_location: anchor.clone(),
            evidence_path: vec![EvidenceStep {
                routine_id: o.id.clone(), // object-level finding (see module doc)
                operation_id: None,
                callsite_id: None,
                loop_id: None,
                source_anchor: anchor,
                note: format!("API page {}", o.name),
            }],
            additional_paths: None,
            affected_objects: vec![o.id.clone()],
            affected_tables: Vec::new(),
            fix_options: vec![FixOption {
                description: "Declare the write surface explicitly: set InsertAllowed/\
                              ModifyAllowed/DeleteAllowed = false on read-only API pages \
                              (and Editable = false), or document the writable intent."
                    .to_string(),
                safety: "high".to_string(),
            }],
            provenance: vec![Evidence {
                source: "tree-sitter".to_string(),
                note: None,
            }],
            actionable_anchor: None,
            fingerprint: None,
            event_kind: None,
            cross_extension_subscribers: None,
        };
        finding.fingerprint = Some(fp_index.fingerprint_of(&finding));
        findings.push(finding);
    }

    findings.sort_by(|a, b| a.id.cmp(&b.id));
    let emitted = findings.len();
    let mut stats = DetectorStats::new(DETECTOR, candidates_considered, emitted);
    stats.add_skip("declaredClosed", skipped_declared_closed);
    Ok(DetectorOutput::no_diag(findings, stats))
}
```

(`FingerprintIndex::fingerprint_of` may assume routine-anchored findings — if it panics/misbehaves on the object-id `enclosing_routine_id`, read `src/engine/l5/fingerprint.rs` and use whatever object-level convention it supports; do NOT skip the fingerprint.)

- [ ] **Step 3: Standard wiring steps** — OPT-IN placement.
- [ ] **Step 4: Golden expectations:** exactly **2 findings** — `"D64 ReadOnly Leaky"` (shape A, `low`, root cause naming `InsertAllowed/ModifyAllowed/DeleteAllowed`) and `"D64 Undeclared"` (shape B, `info`). Stats: `declaredClosed = 1` (`"D64 Closed"`); `"D64 Card"` never a candidate.
- [ ] **Step 5: Boilerplate steps 9–12.** CHANGELOG: `- d64-api-page-write-surface detector (opt-in; BCQuality disable-write-operations-on-read-only-api-pages).` Commit `feat(l5): d64 api-page-write-surface detector (BCQuality wave, opt-in)`.

---

### Task 17: `bcquality` preset + CLI surface

**Files:**
- Modify: `src/engine/gate/presets.rs`
- Modify: `src/bin/alsem.rs` (only if its usage/help text hardcodes the preset list — grep `transaction-integrity` there; `PRESET_NAMES_LIST` may already drive it)
- Test: extend `presets.rs` `#[cfg(test)]`

**Interfaces:**
- Produces: `resolve_preset("bcquality")` → the 13 new names.

- [ ] **Step 1: Write the failing test**

In `presets.rs` tests:

```rust
    #[test]
    fn preset_resolves_bcquality() {
        let names = resolve_preset("bcquality").unwrap();
        assert_eq!(names, PRESET_BCQUALITY);
        assert_eq!(names.len(), 13);
        // every member must be registered
        let all = registered_detectors();
        let registered: std::collections::HashSet<&str> =
            all.iter().map(|d| d.name.as_str()).collect();
        for n in &names {
            assert!(registered.contains(n.as_str()), "{n} not registered");
        }
    }
```

Run: `cargo test -p al_call_hierarchy --lib presets -- --nocapture` → FAIL (no `PRESET_BCQUALITY`).

- [ ] **Step 2: Implement**

```rust
/// The `bcquality` preset — the full BCQuality wave (d52–d64), including its
/// opt-in members (the preset IS the explicit opt-in for them).
pub const PRESET_BCQUALITY: &[&str] = &[
    "d52-bulk-write-param-no-temp-guard",
    "d53-ignored-tryfunction-result",
    "d54-publish-in-tryfunction-cone",
    "d55-event-publish-in-loop",
    "d56-clone-before-write-in-loop",
    "d57-singleinstance-growing-state",
    "d58-query-filter-after-open",
    "d59-integrationevent-var-boolean-guard",
    "d60-upgrade-loop-should-be-datatransfer",
    "d61-ishandled-bypasses-critical-write",
    "d62-telemetry-before-success",
    "d63-html-concat-injection",
    "d64-api-page-write-surface",
];
```

Extend `PRESET_NAMES_LIST` to `&["transaction-integrity", "bcquality"]` and add the `"bcquality"` arm to `resolve_preset`. Update alsem help text if hardcoded.

- [ ] **Step 3: Verify**

Run: `cargo test -p al_call_hierarchy --lib presets` → PASS.
Run: `cargo run --bin alsem -- analyze tests/r0-corpus/ws-d52 --preset bcquality --format json | head -50` (or the alsem invocation shape its usage() documents) → runs, includes d52 findings, exit code per gate policy.

- [ ] **Step 4: Format, lint, CHANGELOG (`- bcquality analyze preset (d52–d64).`), commit**

```bash
rustfmt src/engine/gate/presets.rs
cargo clippy --all-targets --all-features
git add src/engine/gate/presets.rs src/bin/alsem.rs CHANGELOG.md
git commit -m "feat(gate): bcquality preset (d52-d64)"
```

---

### Task 18: Capstone — full validation, CDO/DO measurement, FP triage, docs

- [ ] **Step 1: Full suite**

Run: `cargo nextest run --release` (or `cargo test` if nextest unavailable)
Expected: everything green, including all 13 new golden byte-matches. Any pre-existing golden that changed = investigate root cause before touching it (Working Principle: no blind rebaseline).

- [ ] **Step 2: CDO gate (requires the real workspace)**

Run: `scripts/cdo-gate U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud`
Expected: PASS; north-star metrics untouched (detectors are downstream of resolution — `aldump --program-call-graph-stats` output must stay SHA-identical to `0a3b85bc832ff0a3e77acee118d203edbf62827dc37617c8d9315fe52d5cb7d0`).

- [ ] **Step 3: Measure the new detectors on DO**

```bash
cargo build --release
target/release/alsem analyze "U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud" --format json > demo-out/do-default-post-bcq.json
target/release/alsem analyze "U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud" --preset bcquality --format json > demo-out/do-bcquality.json
```

Record per-detector counts (d52–d64) from `detectorStats`. Compare the default run's PRE-wave baseline (2,282 findings, doc §2) — the delta must be exactly the new default detectors' findings (d52–d60); d61–d64 appear only in the preset run.

- [ ] **Step 4: FP triage of the new findings**

Use the `triage-findings` skill workflow: walk each NEW detector's DO findings (sample ≥ 20 per detector or all if fewer) against real AL source; classify real/FP. Gate: any detector with > 30% FP rate on its sample gets DEMOTED to OPT-IN (move registry entry + name list) with the measured rate recorded in its module doc — precision-first, same bar the existing detectors hold. Re-run Steps 1–3 after any demotion.

- [ ] **Step 5: Documentation**

- Append measured DO counts + triage outcomes to `docs/2026-07-16-scanner-validation-and-bcquality-candidates.md` (new §6 "Wave results").
- CHANGELOG: consolidate the wave under `### Added` if entries sprawl (keep one line per detector).
- CLAUDE.md: no changes needed unless detector counts are quoted there (they are not, verify with a grep for `27 default` — the count lives in the session-notes doc only).

- [ ] **Step 6: Final commit**

```bash
git add docs/2026-07-16-scanner-validation-and-bcquality-candidates.md CHANGELOG.md demo-out/
git commit -m "docs: BCQuality detector wave results (DO counts + triage)"
```

(Check `.gitignore` for `demo-out/` first — if ignored, put the two JSON result files' summary numbers in the doc only and skip staging them.)

---

## Self-review notes (author checklist, run at plan time)

1. **Spec coverage:** all 13 candidates from doc §4 have a task (candidates 1–6 → Tasks 4–9; 7–11 → Tasks 10–13 + 14; 12 → Task 15; 13 → Task 16). Overlap notes from doc §4 honored: d55 documents its d2 boundary; d61 documents its d43 boundary and is OPT-IN. The §4 "ReadIsolation" half of candidate 13 is explicitly scoped out in d64's module doc (not silently dropped).
2. **Type consistency:** all detector fns use the `(resolved: &L3Resolved, ctx: &DetectorContext) -> Result<DetectorOutput, DetectorError>` signature; names in registry/presets/Smoke entries match each task's `DETECTOR` const character-for-character.
3. **Known risks flagged inline:** d61 guard-statement anchor extent (Step 4 stop-condition), d64 fingerprint on object-level findings, d58 query-object parseability, Task 2 object-anchor availability. Each has an explicit verify-or-stop instruction rather than a silent assumption.
4. **Compile-fidelity caveat:** struct field spellings were verified against the sources on 2026-07-16 (commit 6b1d890); if any drift by execution time, align with the d33/d35 templates and the Substrate reference section rather than improvising.
