# Engine Memory/Speed Wave 2b (Trigger-Edge Builder Parity) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the L3 implicit-trigger edge builder the SAME applicability rules the fresh resolver already enforces (field-specific OnValidate targeting + the RunTrigger gate), removing the measured over-approximation that fuses the 8020 corpus's 846-member SCC (97% of whose trigger edges are field-collapsed OnValidate) — the measured root of both the Jacobi plateau and d1's unbounded walk.

**Architecture:** One semantic fix in `src/engine/l3/implicit_edges.rs` + a field-aware symbol lookup, then regen-and-triage (this is the wave's FIRST intentional advisory-graph semantics change: goldens MOVE, findings may move — every movement is triaged, never blind-rebaselined), an FP review on DO, and the payoff measurement (SCC shatter + full-default 8020). The fresh resolver is UNTOUCHED — resolution truth and the north-star SHA cannot move (no `src/program/` file is in scope).

**Tech Stack:** Rust, crate `al-call-hierarchy`. No new dependencies.

## Global Constraints

- Evidence base (read first): `docs/superpowers/specs/2026-07-18-wave2-measurements.md` §2/§4a/§6 + `.superpowers/sdd/w2-trigger-edge-verdict.md` is GONE with the old worktree — its verdict summary lives in the measurements doc §2 "Update — VERIFIED".
- The PARITY ORACLE is `src/program/resolve/applicability.rs:159-227` (`implicit_trigger_route_applicable`): RunTrigger::False → no edge; Insert/Modify/Delete → object-level trigger only (`enclosing_member` None); Validate → the SPECIFIC field's OnValidate (`enclosing_member == field`), never an object-level one. Mirror its semantics; do NOT modify it or anything under `src/program/`.
- Goldens: `scripts/check-goldens --regen` then INSPECT — every changed golden line must be explainable as "edge now correctly dropped/retargeted per the oracle rules". Unexplainable changes = STOP. Then `scripts/check-goldens` green.
- rustfmt per file, never `cargo fmt`. Explicit staging. No stash. No `| tail` on long runs. Clippy clean. Full `cargo test` green after regen.
- DO workspace `U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud`; corpora rebuild script in the Wave-1 plan's Global Constraints.
- Suggested dispatch: T1 Opus, T2 Sonnet, T3 Opus (judgment), T4 Sonnet (runbook), T5 orchestrator. Reviewers: Opus for T1/T3, Sonnet otherwise.

---

### Task 1: Builder parity (field-specific OnValidate + RunTrigger gate)

**Files:**
- Modify: `src/engine/l3/implicit_edges.rs` (module doc + `trigger_mapping` :17-24 + `build_implicit_trigger_edges` :29-70)
- Modify: `src/engine/l3/symbol_table.rs` (add a field-aware trigger lookup next to `routine_in_object` ~:270)
- Test: `implicit_edges` unit tests (same file or the l3 test module — follow where existing implicit-edge tests live: `grep -rn "build_implicit_trigger_edges" src/ tests/`)

**Interfaces:**
- New: `SymbolTable::trigger_in_object(&self, object_id: &str, trigger_name: &str, enclosing_member_lc: Option<&str>) -> Option<&L3Routine>` — name match AND `enclosing_member` match (None = object-level trigger only; Some(f) = the field's own trigger only). Case-insensitive on both (follow the fold conventions already used in symbol_table — `eq_fold_identifier`).
- `build_implicit_trigger_edges` signature unchanged.

- [ ] **Step 1: Discover the exact fresh-side RunTrigger mapping (do not guess).**

The oracle gates on `ctx.run_trigger == RunTrigger::False`. Find how the fresh extractor maps the IR op to that enum: `grep -rn "RunTrigger" src/program/resolve/ | head -20` and read the construction site (likely `extract.rs`). Determine: does an ABSENT run-trigger argument map to False, True, or Unknown for Insert/Modify/Delete, and what does Validate get? Record the answer in your report — the L3 gate below must mirror it EXACTLY. (L3's `PRecordOperation.run_trigger: Option<bool>` — `src/engine/l2/features.rs:267` — is the input; the anatomy sample expected `Insert()`/`Delete()` with absent arg to fire NOTHING, matching AL semantics, but the ORACLE'S mapping is what we mirror, including any conservatism it has.)

- [ ] **Step 2: Failing tests first.**

Write unit tests (RED) pinning the NEW behavior with a small in-memory workspace (follow the existing implicit-edge test fixtures' construction style):
1. `validate_targets_field_specific_trigger`: table with fields A and B, each with an OnValidate member trigger; a routine validates field B → edge must point at B's trigger routine (assert `to == B-trigger id`), not A's.
2. `validate_without_matching_field_trigger_emits_no_edge`: validate field C (no OnValidate on C) while A has one → NO edge.
3. `insert_run_trigger_false_emits_no_edge`: `run_trigger == Some(false)` → no OnInsert edge. Plus the absent-arg case per Step 1's discovered mapping.
4. `insert_object_level_trigger_still_edges`: RunTrigger-true Insert → edge to the table's OnInsert (enclosing_member None), unchanged.
5. `validate_never_targets_object_level_onvalidate`: an object-level OnValidate (no enclosing_member) must NOT be a Validate target.

Run: `cargo test -p al-call-hierarchy --lib implicit_edges` → new tests FAIL.

- [ ] **Step 3: Implement.**

`trigger_in_object` in symbol_table.rs (mirror `routine_in_object`'s lookup shape, adding the `enclosing_member_lc` equality — `None` requires the routine's `enclosing_member.is_none()`, `Some(f)` requires fold-equality with the routine's `enclosing_member`).

In `build_implicit_trigger_edges`, replace the lookup block:

```rust
            // RunTrigger gate — mirror implicit_trigger_route_applicable
            // (applicability.rs:159-167) exactly: <fill per Step-1 finding>.
            if /* op.run_trigger maps to the oracle's False */ {
                continue;
            }
            let trigger = match op.op.as_str() {
                "Validate" => {
                    // Field-specific: the validated field's OWN OnValidate.
                    let Some(field_lc) = op
                        .field_arguments
                        .as_ref()
                        .and_then(|fa| fa.first())
                        .map(|f| f.fold_identifier())
                    else {
                        continue; // Validate with no captured field → no edge (oracle: ctx.field required)
                    };
                    symbols.trigger_in_object(&table_object.id, "OnValidate", Some(&field_lc))
                }
                _ => symbols.trigger_in_object(&table_object.id, trigger_name, None),
            };
            let Some(trigger) = trigger else {
                continue;
            };
```

(Adapt to the real field-name normalization: check how `enclosing_member` stores quoted field names — `l3_workspace.rs` RE-3/RE-4 docs say it's the UNESCAPED logical identifier — and how `field_arguments` entries are stored (quoted?). Normalize BOTH sides the same way; `strip_quotes` + fold as needed. Get this right or field matching silently never matches — test 1 catches it.)

Update the module doc (the "(not captured)" sentence is now false) and `trigger_mapping`'s Resolution values if the oracle's Insert/Modify/Delete story changes them (likely unchanged — "maybe" reflected uncaptured RunTrigger; now captured, a `Some(true)` Insert edge is arguably Resolved — DO NOT change Resolution in this task; note it as a follow-up question instead. Minimal semantic surface.)

- [ ] **Step 4: GREEN + regen + full gates.**

```bash
cargo test -p al-call-hierarchy --lib implicit_edges   # new tests PASS
bash scripts/check-goldens --regen > /tmp/w2b-t1-regen.log 2>&1; echo exit=$?
git diff --stat tests/ | tail -20    # SEE what moved — expect l3eg/r2c edge goldens, possibly cli-a findings
```

INSPECT the regen diff: every removed/retargeted edge line must match the oracle rules (spot-check 10 diffs minimum, cite them in the report). Unexplainable movement = STOP + report. Then:

```bash
bash scripts/check-goldens > /tmp/w2b-t1-goldens.log 2>&1; echo exit=$?  # 0
cargo test > /tmp/w2b-t1-test.log 2>&1; echo exit=$?                     # 0
cargo clippy --all-targets --all-features 2>&1 | grep -c '^error'        # 0
```

- [ ] **Step 5: rustfmt + commit** (implicit_edges.rs, symbol_table.rs, regen'd goldens together — staged explicitly by path):

```bash
git commit -m "fix(l3): implicit-trigger edges gain fresh-resolver applicability parity

Field-specific OnValidate targeting (the validated field's own trigger,
never an arbitrary per-table one) + the RunTrigger gate, mirroring
implicit_trigger_route_applicable (applicability.rs:159-227) with data the
L2 walk already captures (field_arguments, run_trigger). Removes the
measured field-collapse over-approximation (20/20 sampled 846-SCC trigger
edges; 97% OnValidate). Goldens regenerated and per-line triaged against
the oracle rules."
```

---

### Task 2: Downstream-consumer audit + regen coherence

**Files:** none beyond what T1 regenerated — this is a verification task.

- [ ] **Step 1:** `grep -rn "ImplicitTrigger\|implicit-trigger" src/engine/ | grep -v implicit_edges.rs` — list every consumer (combined-graph fold, detectors, event flow, coverage). For each: does the edge-set change require any consumer-side adjustment, or is it purely fewer/retargeted edges? (Expected: pure consumer of the edge list; no code change.)
- [ ] **Step 2:** Confirm `src/program/` untouched: `git diff master --stat -- src/program/` → empty.
- [ ] **Step 3:** Run the l3/differential umbrellas explicitly: `cargo test --test l3 > /tmp/w2b-t2.log 2>&1; echo exit=$?` and `cargo test --test differential >> /tmp/w2b-t2.log 2>&1; echo exit=$?` → 0/0.
- [ ] **Step 4:** Report (no commit unless something needed fixing — then it belongs to T1's scope, send it back).

---

### Task 3: DO findings differential + FP triage

**Files:** report `.superpowers/sdd/w2b-do-triage.md`.

- [ ] **Step 1:** Build release-fast at T1's commit AND at its parent; run `alsem analyze <DO> --format json` with both; extract findings arrays; diff.
- [ ] **Step 2:** For EVERY changed finding (added/removed/moved): open the DO source, judge — improvement (edge was false, finding was FP or wrong-cone), regression (real finding lost), or neutral (id/witness drift). d1/d8/d50 (span+cone consumers) and d43/d44/d45 are the likely movers.
- [ ] **Step 3:** ANY regression = STOP, report BLOCKED with the case. Otherwise report the triage table. Expected honest outcome: few-to-zero changes on DO (551 files, small SCCs) — the fix targets Base-App-scale density; zero DO movement is a PASS, not a failure to verify.

---

### Task 4: Payoff measurement

**Files:** report `.superpowers/sdd/w2b-payoff.md`. Corpora per Wave-1 plan script.

- [ ] **Step 1:** Re-add the MINIMAL SCC stats probe as a throwaway WIP commit (the Wave-2 SCCSTATS/SCCANATOMY block — recover with `git show c8836e7^:src/engine/l5/detector_context.rs` history or re-write ~30 lines; env-gated; marked "WIP(probes) — DROP before merge").
- [ ] **Step 2:** 8020 corpus, `ALSEM_STAGE_TIMING`-less quick anatomy: run with `ALSEM_EXIT_AFTER_SCCSTATS=1` equivalent env from the probe; record SCCSTATS/SCCANATOMY. SUCCESS BAR (from the verdict's estimate): max_scc collapses from 846 toward the ~84-order scale; implicit-trigger intra-share collapses.
- [ ] **Step 3:** Full-default 8020 run, detached monitor, 2h cap: does it FINISH now? Record wall/RSS/top detectors. 5400 full-default + 8020 3-det + DO timing for the regression rows.
- [ ] **Step 4:** Report all numbers honestly, including any that miss the bar.

---

### Task 5: Capstone — docs + handoff

- [ ] Fold T1-T4 into the measurements doc ("Wave-2b outcome"), CHANGELOG (`Changed` + the semantics note: advisory implicit-trigger edges now oracle-parity; goldens rebaselined with per-line triage), OUTSTANDING (tick (1); update (2)/(3) per T4's numbers — if full-default 8020 now finishes, d1's §7 redesign may be demoted/closed; B1/B2 re-scoped against the new plateau).
- [ ] Commit docs. Probe sweep + merge decision with the user (Wave-1 procedure). User-run `scripts/cdo-gate` recommended after merge (advisory-graph change: CDO ratchets + detector counts should be re-confirmed).

---

## Self-review notes

- The oracle is mirrored, not extended: table-EXTENSION trigger targeting (which the oracle also allows) is NOT added — the current builder never targeted extension triggers either, so parity-of-restriction only REMOVES false edges; extension-trigger support is noted as a candidate follow-up in T5's OUTSTANDING update, not silently added.
- Resolution values (Resolved/Maybe) deliberately unchanged in T1 — flagged as follow-up question, keeping this wave's semantic surface minimal.
- The regen is triaged, never blind: T1 Step 4's 10-diff minimum + T3's per-finding DO triage are the honesty gates.
