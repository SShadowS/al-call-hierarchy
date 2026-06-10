# Engine fix-then-freeze: trigger enclosing-member + opt-in evidence — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use `- [ ]` checkboxes.

**Goal:** Parity-safely expose, for AL field/control trigger routines, the enclosing-member identity (+ originating object + member-wrapper range), plus an opt-in `analyze --with-evidence` projection — so al-perf P3.2 can precisely attribute CPU-profile field-trigger frames. NO StableRoutineId change, NO default/parity-locked output change.

**Architecture:** E1 captures the data at L3 assembly (additive struct fields). E2 surfaces it on the Rust-only inventory projection. E3 surfaces evidencePath + a POSITION-derived finding-side member discriminator behind a new opt-in flag. E4 proves parity + freezes.

**Tech Stack:** Rust, `cargo test` (the byte-parity differential harness against al-sem TS goldens), tree-sitter-al. Branch `engine` (LOCAL — **NEVER push**).

**Governing spec:** `docs/superpowers/specs/2026-06-10-engine-trigger-member-and-evidence-design.md` — implement to **Revision 2** (RE-1..RE-9).

**NON-NEGOTIABLE parity gates (every task):**
- After each task run the FULL suite: `cargo test`. It MUST stay green and `KNOWN_DIVERGENCES.json` MUST stay `[]`. A moved golden = STOP and report.
- NEVER change `StableRoutineId`/`compute_routine_id`/`canonical_routine_signature` (would rebaseline every golden).
- NEVER edit the L2 copies of `collect_routine_nodes` (`l2/mod.rs:468`, `l2/l2_workspace.rs:441`, `l2/operation_order.rs:860`, `l2/control_context.rs:696`) — ONLY the `l3_workspace.rs` copy.
- NEVER push the `engine` branch.

---

## Task 1 (E1): Capture enclosing-member + originating-object + wrapper-range at L3

**Files:**
- Modify: `src/engine/l3/l3_workspace.rs` (`collect_routine_nodes` ~588-619, the assembly loop ~728-763 + ~859, `L3Routine` ~227-321)
- Test: a new Rust test (e.g. `tests/cli_p1_enclosing_member.rs`) + an inline routine-set/order invariant assert

Satisfies RE-2 (wrapper range), RE-3 (name-field derivation + parent set), RE-4 (unescaped logical name), RE-5 (originatingObject = object decl in scope), RE-7 (only the l3_workspace copy; `node.parent()` at match point), RE-9 (order invariant).

- [ ] **Step 1: Read & verify the real shapes.** Read `L3Routine` (~227-321), `collect_routine_nodes` (~588-619), the assembly loop (~728-763, esp. the `child_by_field_name("name")` read ~741 and `source_anchor = anchor_from_node(routine, …)` ~859), `strip_quotes`, `node_text`, `anchor_from_node`, and how `object_id`/the object decl StableObjectId is in scope (~643, `to_stable_object_id`). Confirm `PAnchor`'s field shape (for the wrapper range).

- [ ] **Step 2: Add fields to `L3Routine`** (additive — these do NOT change serialization; `L3Routine` is not `derive(Serialize)` — verify):
```rust
/// Field/control/action/dataitem member name for a member-trigger routine (unescaped
/// logical identifier); None for procedures & object-level triggers. (RE-3/RE-4)
pub enclosing_member: Option<String>,
/// StableObjectId of the object that DECLARES this trigger (the extension, for an
/// extension-declared trigger). Honest metadata; profile-unjoinable for multi-ext. (RE-5)
pub originating_object: Option<String>,
/// Source range of the member WRAPPER node (field_declaration/page_field/action_declaration/
/// report_dataitem) — the boundary the finding-side discriminator matches against. (RE-2)
pub enclosing_member_range: Option<crate::engine::l2::features::PAnchor>,
```

- [ ] **Step 3: Capture the parent at the DFS match point (RE-7).** In `collect_routine_nodes` (l3_workspace.rs copy ONLY), at the point a `procedure`/`trigger_declaration` node matches, also record `node.parent()` — change the return type to `Vec<(Option<Node>, Node)>` (parent, routine). Do NOT restructure the stack/push order (preserves traversal). The single caller (~728) destructures the pair.

- [ ] **Step 4: Derive the member (RE-3/RE-4).** Add a helper:
```rust
/// Returns the member name for a trigger whose immediate parent is a member-bearing wrapper
/// (any parent that is NOT the object decl and exposes a `name` field): field_declaration,
/// page_field, action_declaration, report_dataitem, query_dataitem. None otherwise.
fn enclosing_member_of(parent: Option<Node>, source: &str) -> Option<(String, /*wrapper node*/ Node)> {
    let p = parent?;
    // object-level triggers: parent is the object decl / object body — no member.
    let name_node = p.child_by_field_name("name")?;     // RE-3: NOT "first child"
    let raw = node_text(name_node, source);
    Some((unescape_al_identifier(strip_quotes(raw)), p)) // RE-4: unescaped logical name
}
```
Implement `unescape_al_identifier` to turn an inner `""` into `"` (RE-4). Restrict to the wrapper kinds via the "has a `name` field AND is not the object decl" rule — verify the object decl node kind so it's excluded (the object's own `name` must NOT become a member). `actionref_declaration` uses `promoted_name` (no `name`) → naturally `None`.

- [ ] **Step 5: Populate in the assembly loop.** For each `(parent, routine)`: set `enclosing_member` + `enclosing_member_range` (from the wrapper node via `anchor_from_node`) when `enclosing_member_of` returns `Some`, else `None`; set `originating_object` = the StableObjectId of the object decl in scope (~643). Procedures/object-level triggers → all `None`.

- [ ] **Step 6: Routine set/order invariant test (RE-9).** Add a test that runs the L3 assembly on a multi-trigger fixture and asserts `workspace.routines` count + the `(id, source_anchor.start_line)` sequence is identical to a frozen expectation (proves the `collect_routine_nodes` change didn't perturb traversal). Plus an enclosing-member unit assert: a two-field-`OnValidate` table fixture → two routines with the SAME `stable_routine_id` but DISTINCT `enclosing_member` + distinct `enclosing_member_range`.

- [ ] **Step 7: PARITY GATE.** `cargo test` — FULL suite green, `KNOWN_DIVERGENCES.json` == `[]`. If any golden moved, STOP (the additive fields must not move bytes; a move means a traversal side-effect — diagnose, don't rebaseline).

- [ ] **Step 8: Commit.**
```bash
git add src/engine/l3/l3_workspace.rs tests/cli_p1_enclosing_member.rs
git commit -m "feat(engine-e1): capture trigger enclosing-member + originating-object + wrapper-range at L3 (additive, parity-safe)"
```

---

## Task 2 (E2): Inventory projection fields + 3-key sort + schema 1.1.0

**Files:**
- Modify: `src/engine/l5/snapshot_full.rs` (`build_inventory_doc` ~1063-1080, `INVENTORY_SCHEMA_VERSION` ~1036)
- Test: `tests/cli_p1_inventory.rs` (~92-94 the version assertion + new field/order assertions)

Satisfies RE-6 (3-key case-insensitive sort), schema bump. Rust-only (no parity surface).

- [ ] **Step 1: Emit the fields.** In `build_inventory_doc`'s row map (~1063-1076), after `routineName` and before `stableRoutineId`, conditionally insert `enclosingMember` (when `r.enclosing_member.is_some()`) and `originatingObject` (when `r.originating_object.is_some()`).

- [ ] **Step 2: Three-key sort (RE-6).** Change the row sort (~1079) from `locale_compare(&a.0, &b.0)` to: primary `locale_compare(stableRoutineId)` → secondary `case_insensitive_compare(enclosingMember)` (None sorts consistently, e.g. before Some) → tertiary `locale_compare(originatingObject)`. Implement `case_insensitive_compare` (or reuse an existing one — grep). This makes duplicate-stableRoutineId rows content-stable regardless of developer casing.

- [ ] **Step 3: Bump schema.** `INVENTORY_SCHEMA_VERSION` `"1.0.0"` → `"1.1.0"` (~1036) with a comment noting the additive enclosingMember/originatingObject fields.

- [ ] **Step 4: Update `cli_p1_inventory.rs`.** Flip the version assertion (~92-94) to `"1.1.0"`; add assertions: the two-field fixture emits two rows with the same `stableRoutineId` but distinct `enclosingMember`, in deterministic (case-insensitive member) order; an object-level trigger row has NO `enclosingMember` key.

- [ ] **Step 5: PARITY GATE.** `cargo test` full green, KNOWN_DIVERGENCES `[]`. (The inventory is Rust-only — the fingerprint differential runs `inventory_only:false` so it's untouched; confirm the differential suites are unchanged.)

- [ ] **Step 6: Commit.**
```bash
git add src/engine/l5/snapshot_full.rs tests/cli_p1_inventory.rs
git commit -m "feat(engine-e2): inventory enclosingMember/originatingObject + 3-key sort, schema 1.1.0 (Rust-only)"
```

---

## Task 3 (E3): `analyze --with-evidence` opt-in (evidencePath + position-derived finding discriminator)

**Files:**
- Modify: `src/bin/alsem.rs` (`AnalyzeCli` ~289-355), the `AnalyzeArgs` struct + `run.rs` (~176-373, the analyze run + JSON build), `src/engine/gate/format_json.rs` (~109-220 finding emission + envelope ~304-332), `src/engine/gate/projection.rs` (the `routineId→routine`/position index, ~55-120)
- Test: `tests/cli_a_with_evidence.rs` (new) + confirm `tests/cli_a_json_differential.rs` stays green

Satisfies RE-1 (position-based discriminator), RE-2 (wrapper-range containment), RE-7 (post-projection attach; flag literals), RE-8 (schemaVersion conditional).

- [ ] **Step 1: Add the flag.** `AnalyzeCli`: `#[arg(long = "with-evidence", default_value_t = false)] pub with_evidence: bool`. Add `with_evidence: bool` to `AnalyzeArgs`; set `with_evidence: false` at EVERY test literal (`cli_a_json_differential.rs:129,478` + any others — grep `AnalyzeArgs {`). Thread it to the JSON build (`JsonFormatInputs`).

- [ ] **Step 2: Position-derived discriminator (RE-1/RE-2).** Build, from the resolved routines, an index of member-trigger routines as `(source_unit_id, wrapper_range, enclosing_member, originating_object)`. For each finding, when `--with-evidence`, find the member whose `(source_unit_id, enclosing_member_range)` CONTAINS the finding's `primary_location` (start position), smallest containing range; attach `enclosingMember`/`originatingObject` to that finding's emitted `primaryLocation`. Returns `None`/omitted when no container (object-level/procedure findings) — engine never throws. Do NOT use `enclosingRoutineId` (it's collapsed — RE-1).

- [ ] **Step 3: evidencePath, post-projection (RE-7).** Where `run.rs` (~364) has `paired: (FindingSummary, &Finding)`, surface `Finding.evidence_path` (`finding.rs:96`) into the JSON ONLY under `--with-evidence`. The evidence step `routineId` uses the `:`-form via the same internal→stable map the gate already applies (`map_routine_id`). Emit `evidencePath: StableEvidenceStep[]` (routineId/sourceAnchor/note + optional operationId/callsiteId/loopId) on each finding object in `format_json.rs`, gated. Prefer NOT adding a field to `FindingSummary` (struct-literal at ~7 sites); attach via the paired `&Finding` at the JSON build. If a `FindingSummary` field is unavoidable, make it `Option` + `skip_serializing_if` + `None` at all literals.

- [ ] **Step 4: schemaVersion (RE-8).** Default (no flag): envelope `schemaVersion` stays `"1.0.0"` (the differential asserts it at `cli_a_json_differential.rs:394`). Under `--with-evidence`: emit `"1.1.0"`. The default output (no flag) must be byte-identical — all new fields conditional/absent.

- [ ] **Step 5: Tests.** `tests/cli_a_with_evidence.rs`: the two-field-`OnValidate` fixture under `--with-evidence` → EACH finding carries its OWN `enclosingMember` (position-derived, per-FINDING — RE-1), `evidencePath` present; the SAME fixture under plain `analyze` → output byte-identical to its existing golden (no evidencePath/enclosingMember keys), `schemaVersion "1.0.0"`. A finding outside any trigger → no `enclosingMember`.

- [ ] **Step 6: PARITY GATE.** `cargo test` full green, KNOWN_DIVERGENCES `[]`. CRITICAL: `cli_a_json_differential.rs` (the analyze byte-parity harness, which never passes `--with-evidence`) MUST be byte-identical — proving the default output is unchanged.

- [ ] **Step 7: Commit.**
```bash
git add src/bin/alsem.rs src/engine/gate/format_json.rs src/engine/gate/projection.rs tests/cli_a_with_evidence.rs <run.rs path> <any AnalyzeArgs literal test files>
git commit -m "feat(engine-e3): analyze --with-evidence (evidencePath + position-derived member discriminator), default byte-identical"
```

---

## Task 4 (E4): Prove + freeze

**Files:** version-tuple/cache tests if needed; a freeze note in `docs/engine-migration.md`.

- [ ] **Step 1: Full parity + new-feature suite.** `cargo test` — ALL green; `KNOWN_DIVERGENCES.json` == `[]`. Confirm: the analyze differential, the fingerprint differential, the cache differential are all byte-identical; the new E1/E2/E3 tests pass.

- [ ] **Step 2: Cache-tuple check (RE-8).** Verify `cli_c_cache_differential.rs` (~347-354) is unaffected (INVENTORY_SCHEMA_VERSION + analyze schemaVersion are NOT in the tuple). If the engine has a separate version assertion that includes the inventory schema, update only that (not a parity golden).

- [ ] **Step 3: Freeze note.** Append to `docs/engine-migration.md`: the frozen schema al-perf P3.2 consumes — inventory 1.1.0 (`enclosingMember`/`originatingObject`), `analyze --with-evidence` (1.1.0: `evidencePath` + per-finding `enclosingMember`/`originatingObject`), the position-based discriminator semantics, and the documented limitation (originatingObject profile-unjoinable → multi-extension stays ambiguous; al-perf joins enclosingMember CASE-INSENSITIVELY per RE-4).

- [ ] **Step 4: Commit.**
```bash
git add docs/engine-migration.md <any version test>
git commit -m "chore(engine-e4): prove parity + freeze trigger-member/evidence schema for al-perf P3.2"
```

---

## Self-Review
- **Spec coverage:** E1 (Task 1: RE-2/3/4/5/7/9), E2 (Task 2: RE-6 + schema), E3 (Task 3: RE-1/2/7/8), E4 (Task 4: parity proof + RE-5 doc).
- **Parity spine:** every task ends with the full `cargo test` gate + KNOWN_DIVERGENCES `[]`; no StableRoutineId change; only the l3_workspace `collect_routine_nodes` copy; default outputs byte-identical (E1 additive, E2 Rust-only, E3 opt-in).
- **The crux (RE-1):** finding discriminator is POSITION-based (wrapper-range containment), never the collapsed `enclosingRoutineId`; a per-FINDING test proves two fields' findings get distinct members.
- **Honesty:** profile-unjoinable `originatingObject` is documented as such (not claimed to resolve multi-extension); engine never throws (all `None`-degrading).
- **Type consistency:** the L3 `enclosing_member`/`originating_object`/`enclosing_member_range` triplet flows L3 → inventory (E2) → finding discriminator (E3, via wrapper-range match); `enclosingMember` is the unescaped logical name everywhere.
