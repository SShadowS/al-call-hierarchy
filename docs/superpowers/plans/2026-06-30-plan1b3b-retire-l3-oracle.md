# Plan 1B.3b — Retire the L3 oracle from the fresh resolver's validation gates Implementation Plan

> Status: v2 — rewritten after gpt-5.5 GO-WITH-CHANGES + gemini-3.1-pro NO-GO. Convergent fixes: (1) a
> digest-only CDO golden GUTS the post-L3 floor → instead ANONYMIZE (stable-hash the proprietary site-keys
> + target-ids into opaque deterministic ids) so the FULL ~13k per-site CDO golden is COMMITTABLE without
> leaking customer names — a regression shows as a reviewable anonymized ±edge diff; (2) the applicability
> teeth verify SOUNDNESS (route well-formed) NOT COMPLETENESS/EXACTNESS (the right N targets) — so deleting
> the ImplicitTrigger/EventFlow dual-run gates loses real coverage → EXTEND the freeze to capture L3's
> ImplicitTrigger + EventFlow TARGET SETS as frozen baselines (option a), keep+convert
> `event_fixture_two_stage_join`, teeth = soundness-only; (3) env-gated skip is fail-OPEN → an
> `ENFORCE_CDO_WS` flag PANICS for gated runs if the workspace/golden is missing or the diff didn't run, +
> public CI validates committed metadata; (4) move L3-minting to a dev-only tool so the gate module is
> L3-free post-removal; (5) port the teeth + tests BEFORE deleting the only live copy.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Make the fresh AL/Business-Central call-graph resolver **stand alone** — freeze the L3-derived correctness baselines as COMMITTED ANONYMIZED frozen goldens (covering ALL dispatch kinds, incl. ImplicitTrigger + EventFlow), then remove the L3 oracle (`project_l3*`, `assemble_and_resolve_workspace_default`, the event-graph walk) from the dual-run validation gates in `differential.rs`, leaving the engine validated by the frozen goldens + its L3-INDEPENDENT contracts (the teeth as soundness checks). L3 (`src/engine/l3`) STAYS as the `aldump` analysis backbone; minting moves to a dev-only tool so the gate path is L3-free. No LSP cutover.

**Architecture:** Surgical removal + the C1 freeze, in the safe ORDER: (1) freeze ALL dispatch kinds as committed ANONYMIZED goldens + the `ENFORCE_CDO_WS` hard-fail guard + verify frozen==live; (2) port the fan-out applicability teeth into the L3-independent `route_applicability` + prove the replacement green BEFORE (3) removing `project_l3*` + deleting the L3-comparison gates; (4) verify L3-free + honest CHANGELOG. Doing removal before freeze, or deleting a gate before its frozen-baseline + ported teeth are green, evaporates the floor silently (env-gated tests skip, not fail) — THE biggest risk.

**Tech Stack:** Rust (edition 2024, toolchain 1.96.0). L3-minting moves to a DEV-ONLY tool (an `xtask`/dev binary/test-helper OUTSIDE `src/program/resolve`) — the LAST oracle use; after removal the `src/program/resolve` gate path has NO `engine::l3`/`engine::l2` import EXCEPT the sanctioned `builtins.rs::global_builtins` platform-DATA catalog.

**Source of truth:** the 1B.3b grounding map (this session) + the spec §6 + the opus C1 finding. Read first.

## Key facts grounding this plan (all file:line verified)

- **Removal surface (`differential.rs`):** 6 oracle fns — `project_l3` (:301), `project_l3_sites` (:431), `project_l3_in_scope` (:1190), `project_l3_member_in_scope` (:1744), `project_l3_implicit_trigger_in_scope` (:2544), the L3-walk in `run_event_flow_gate` (:3235-3736) — plus `semantic_golden.rs::run_cdo_semantic_audit` `project_l3` (:585). Imports: `engine::l3::{call_resolver,l3_workspace,symbol_table,taxonomy,event_graph}` + `engine::l2::{features,ir_walk}`.
- **L3-INDEPENDENT survivors (the post-L3 gate):** `evidence_overclaim`/`witness_contract_holds` (:1120), the `fresh_ahead_*`/`unverified_extra` TEETH (in the gates' FreshOnly branches — must be PORTED), the semantic golden (once frozen), `route_applicability`/`ApplicabilityReport`, `full.rs` `coverage_holds`/`Histogram`/`abi_ingestion_integrity` (ZERO `engine::l3` imports — the template). The fixture golden (Test 14, `run_semantic_diff(fixture, &golden)`) is the already-frozen load pattern.
- **Serde confirmed:** `SemanticGolden`/`GoldenSiteKey`/`GoldenTarget`/`GoldenEntry` derive `Serialize`/`Deserialize`.
- **THE COVERAGE GAPS (confirmed):** the golden (built from `project_l3`/`resolve_calls`) CANNOT capture ImplicitTrigger (L3 keys by `PRecordOperation.id`, disjoint from `PCallSite.operation_id` → silently dropped from `project_l3`'s `callsite_by_id.get(...)?`) NOR EventFlow (separate `build_event_graph`/`project_event_graph` subsystem). → the freeze MUST be EXTENDED to these two dispatch kinds as their OWN frozen baselines (their native id namespaces), else deleting `run_implicit_trigger_harness`/`run_event_flow_gate` loses TARGET-SET correctness (the teeth only check soundness, not completeness).
- **L3 module KEEP:** `src/engine/l3`(+l2/l4/l5/gate) = aldump backbone; `builtins.rs::global_builtins` (data catalog) + `--l3-call-graph-stats` STAY (out of scope).

## The anonymization (the key fix — makes the CDO floor a committable runnable artifact)

CDO is a real customer workspace; the full golden's site-keys (file/line/proc) + target-ids leak proprietary names. **FIX: anonymize.** During minting (the dev tool), pass every identifying string (file path, routine name, object name/number, callee fingerprint, target object/proc) through a STABLE keyed hash → opaque deterministic ids. The anonymized golden encodes the GRAPH STRUCTURE (which site resolves to which target id, which dispatch kind, the genuine_wrong/fresh_ahead classification) WITHOUT the names. Committable (`tests/goldens/semantic-edges/cdo-anon.json`, minified); a NEW confidently-wrong edge shows as a reviewable anonymized ±diff. The SAME anon runs fresh-side at audit time so the diff aligns. **Public CI** lacks the CDO source to regenerate the fresh IR → it validates the committed golden's METADATA only; the per-site diff runs on the INTERNAL/gated runner (with the CDO source). Honest: the CDO-scale per-site floor is active on the gated runner, not public CI.

Three implementation rules (round-2 reviewers):
1. **Versioned DOMAIN-SEPARATED anon** — `anon("site:v1", canonical_site_key)`, `anon("target:v1", canonical_target_signature)`, `anon("trigger-op:v1", canonical_record_op_key)`, `anon("event-pair:v1", pub_key + "->" + sub_key)` — different namespace tags so the SAME string under different roles yields DIFFERENT ids (no cross-namespace collision); the `:v1` enables reviewable future migrations.
2. **Keep non-sensitive labels in CLEARTEXT** in the golden — dispatch kind, route family, evidence/confidence bucket, the genuine_wrong/fresh_ahead classification, schema version, per-dispatch-kind counts — so the anonymized diff has SEMANTIC anchors (only the identifying strings are hashed).
3. **The de-anon MAP (gemini's required actionability fix)** — an all-opaque-hashes diff is un-actionable. The dev-mint tool (AND the fresh-side audit on FAILURE) writes a GITIGNORED local `tests/goldens/semantic-edges/cdo-deanon-map.json` (`AnonId → "file/line/object/proc plaintext"`). The committed golden stays pure-anon; a dev with CDO access reverses a failing diff's anon-ids via the local map to the exact broken AL code. Without it, a gated-run alarm is structurally caught but blind.
- **Privacy/governance note:** a FIXED COMMITTED salt is deterministic but weak vs a dictionary attack on common AL object/proc names. Use HMAC with a NON-COMMITTED key (stored in the gated runner's secret store + the dev's local env) IF data-governance requires adversarial-reversal resistance; a fixed salt is adequate purely for DIFFING. Resolve this as a data-governance decision in Task 1 Step 1 (default: HMAC non-committed key — safest; the committed golden is still diffable, the key only gates de-anonymizability).

## Global Constraints

- Rust edition 2024; toolchain 1.96.0. `rustfmt <file>` per-file — never `cargo fmt`. Stage only named files — never `git add -A`.
- CI gates: `cargo clippy --release --all-features -- -D warnings` (NO `--tests`), `cargo fmt --check`, `cargo test --workspace` (fixture-only, no `CDO_WS` — fully green WITHOUT the oracle/workspace). All pass.
- **FREEZE-AND-PROVE BEFORE REMOVE (the spine):** Task 1 freezes ALL dispatch kinds (anonymized, committed) + verifies frozen==live while L3 present; Task 2 ports the teeth + proves the replacement green; Task 3 removes L3 only after. Never delete a gate before its frozen baseline + ported teeth are green.
- **No fail-OPEN skip:** env-gated CDO tests may skip when `CDO_WS` absent, BUT an `ENFORCE_CDO_WS=1` flag (for the gated/internal runner) makes a missing `CDO_WS`, a missing/invalid frozen golden, or a `checked_sites==0` audit a HARD PANIC. Public CI validates the committed golden's metadata (exists, schema, entry-count, genuine_wrong=42) unconditionally.
- **L3-minting OUT of the gate path:** the L3-mint code lives in a dev-only tool (`xtask`/dev bin/test-helper outside `src/program/resolve`); the gate module only LOADS frozen artifacts → `src/program/resolve` is L3-free post-removal (except `builtins.rs::global_builtins`).
- **No new resolution work / no disambiguation:** scoped to freeze + L3-gate removal + the teeth port. genuine_wrong=42 adjudication, fresh⊆l3 recall, Cat-D dispatch, the 14 fresh_missing, the snapshot double-include — OUT of scope (post-1B.3b).
- **L3 module + builtins.rs + aldump flags stay.** **No silent coverage loss:** ImplicitTrigger/EventFlow get EXTENDED frozen baselines (not delete-and-rely-on-teeth); the teeth are documented as SOUNDNESS checks, not equivalence.
- Determinism (frozen-load run-to-run identical); honest CHANGELOG (NOT "self-validated from first principles" — validated by frozen historical L3 verdict + L3-independent contracts + synthetic fixtures + the gated CDO artifact).

## File / module structure

| File | Responsibility |
|------|----------------|
| `xtask/` or `src/bin/mint-goldens.rs` (create) | Task 1: the DEV-ONLY L3-minting tool (anonymized goldens for all dispatch kinds) — the only L3 consumer after removal. |
| `src/program/resolve/anon.rs` (create) | Task 1: the stable-hash anonymization (shared by mint + audit). |
| `src/program/resolve/semantic_golden.rs` (modify) | Task 1: `run_cdo_semantic_audit` load-frozen + anon + ENFORCE guard; Task 2: port the fan-out teeth into `route_applicability`. |
| `tests/goldens/semantic-edges/` (modify) | Task 1: committed anonymized CDO golden + ImplicitTrigger + EventFlow frozen baselines. |
| `src/program/resolve/differential.rs` (modify/shrink) | Task 1: capture the ImplicitTrigger/EventFlow frozen baselines; Task 3: remove the 6 oracle fns + imports + gut the L3-comparison gates + trim structs. |
| `tests/program_resolve_harness.rs` (modify) | Tasks 1-3: frozen-load gates + the ENFORCE guard; delete the L3-comparison gate tests (after the frozen replacements are green). |
| `CHANGELOG.md` + charter memory (modify) | Task 4. |

---

### Task 1: Freeze ALL dispatch kinds as committed ANONYMIZED goldens + dev-mint tool + ENFORCE guard

**Files:** Create `src/program/resolve/anon.rs` + the dev-mint tool; modify `semantic_golden.rs`, `differential.rs` (capture ImplicitTrigger/EventFlow baselines), `tests/program_resolve_harness.rs`, `tests/goldens/semantic-edges/**`; Test (CDO-mint + frozen-load + ENFORCE).

**Interfaces:** `anon.rs`: `anon_id(s: &str) -> AnonId` (stable keyed hash, fixed committed salt). The dev tool: mint the anonymized goldens for Member/Interface (the semantic golden), ImplicitTrigger (native `PRecordOperation` keys, anonymized), EventFlow (`project_event_graph` pairs, anonymized).

- [ ] **Step 1: `anon.rs`** — the stable anonymization, DOMAIN-SEPARATED + versioned: `anon(domain: &str, s: &str) -> AnonId` (HMAC with a non-committed key by default — the data-governance decision; deterministic; same fn at mint AND audit time). Domains: `site:v1`/`target:v1`/`trigger-op:v1`/`event-pair:v1`. Unit tests: same (domain,input) → same AnonId; same input under DIFFERENT domains → DIFFERENT ids (collision/domain test); a 2×-mint of a small synthetic workspace → byte-identical minified JSON (anon determinism). Decide + record the salt-vs-HMAC-key governance choice here.
- [ ] **Step 2: The dev-mint tool** (`xtask`/`src/bin/mint-goldens.rs`, OUTSIDE `src/program/resolve`): under it (the LAST L3 use), mint + ANONYMIZE + write the committed goldens (non-sensitive labels in cleartext, identifying strings hashed per the domain-separated scheme): (a) the Member/Interface semantic golden (`cdo-anon.json`); (b) the ImplicitTrigger frozen baseline (`project_l3_implicit_trigger_in_scope`'s L3 target set, native `PRecordOperation`-keyed, anonymized — `cdo-trigger-anon.json`); (c) the EventFlow frozen baseline (`project_event_graph` pub→sub pairs, anonymized — `cdo-event-anon.json`). Commit all three minified. ALSO write the GITIGNORED `cdo-deanon-map.json` (`AnonId → plaintext`) for local root-causing. (This is where ALL `project_l3`/`build_event_graph` calls live post-1B.3b.)
- [ ] **Step 3: Swap the audits to load-frozen + anon.** `run_cdo_semantic_audit` (+ new `run_cdo_trigger_audit`/`run_cdo_event_audit`): on a normal run, LOAD the committed anonymized golden + anonymize the FRESH edges with the same `anon_id` → diff per-site. NO `project_l3` call in the gate module. The ENFORCE guard: `CDO_WS` absent + `ENFORCE_CDO_WS` unset → skip; `ENFORCE_CDO_WS=1` + (CDO_WS missing OR golden missing/invalid OR checked_sites==0) → PANIC. Public-CI (no CDO) always validates the committed golden's metadata (schema, entry-count, genuine_wrong=42).
- [ ] **Step 4: Keep + convert `event_fixture_two_stage_join`** (the always-run non-proprietary fixture) to a frozen/hand-authored expected-output test (it currently calls `build_event_graph` live — convert to assert fresh vs a committed fixture EventFlow baseline). Add an ImplicitTrigger TARGET-SET fixture (synthetic, committed, asserts fresh resolves the exact trigger set). These survive oracle retirement as the always-run L3-independent semantic coverage for those dispatch kinds.
- [ ] **Step 5: Verify frozen==live** (WHILE L3 present, via the dev tool): regen all three goldens, run the frozen-load audits → IDENTICAL classification to the last live run. Run WITH `CDO_WS` + `ENFORCE_CDO_WS=1`.
- [ ] **Step 6: rustfmt + clippy + `cargo test --workspace` (no CDO) + the gated CDO audit + commit** — `feat(resolve): committed anonymized frozen goldens (all dispatch kinds) + dev-mint tool + ENFORCE guard (1B.3b Task 1)`.

---

### Task 2: Port the fan-out applicability teeth (soundness) + prove the replacement green

**Files:** Modify `semantic_golden.rs` (`route_applicability`), `tests/program_resolve_harness.rs`; Test (fixture + CDO). **This MUST precede the gate deletion (Task 3) — port + prove before deleting the only live copy.**

**Interfaces:** `route_applicability` gains the four fan-out predicate checks over EVERY fan-out route (extracted from the gates' FreshOnly branches BEFORE those gates are deleted).

- [ ] **Step 1: Write failing tests** — a fixture with Interface/instance-builtin/ImplicitTrigger/EventFlow routes → `route_applicability` checks every Interface route passes `interface_route_applicable`, instance-builtin passes `instance_builtin_route_applicable`, ImplicitTrigger passes `implicit_trigger_route_applicable`, EventFlow passes `verify_event_subscriber_route` — `violations==0`; a fabricated non-applicable fan-out route → a violation. Document these as SOUNDNESS checks (the route is well-formed/applicable) — NOT completeness (the frozen baselines + fixtures from Task 1 carry target-set completeness/exactness).
- [ ] **Step 2: Run — fail.**
- [ ] **Step 3: Implement** — extract + port the four predicates (from `applicability.rs` + `verify_event_subscriber_route`) into `route_applicability`, running over `resolve_full_program`'s FULL edge set by `EdgeKind`/`DispatchShape`. Confirm green on the fixture + (CDO) `violations==0`.
- [ ] **Step 4: Run** the fixtures + (WITH CDO_WS) `route_applicability` → `violations==0`, deterministic.
- [ ] **Step 5: Full gate + commit** — `feat(resolve): port fan-out applicability teeth (soundness) into route_applicability (1B.3b Task 2)`.

---

### Task 3: Remove `project_l3*` from the gates + trim the L3-comparison structs + confirm L3-free

**Files:** Modify `differential.rs` (remove the 6 oracle fns + imports + the L3-comparison gate bodies + structs), `tests/program_resolve_harness.rs` (delete the L3-comparison gate tests — their replacements are green from Tasks 1-2); Test (fixture-only green).

- [ ] **Step 1: Confirm the replacements are green** (Tasks 1-2): the frozen goldens (all dispatch kinds) + the ported teeth + the converted fixtures cover what the dual-run gates validated. List each deleted gate ↔ its replacement (Bare/Member → frozen semantic golden + coverage + teeth; ImplicitTrigger → frozen trigger baseline + fixture + teeth; EventFlow → frozen event baseline + converted fixture + teeth).
- [ ] **Step 2: Remove** the 6 oracle fns + ALL `engine::l3`/`engine::l2` imports they pulled in; delete `run_harness`/`run_site_harness`/`DiffReport` + the L3-comparison cores of the four harnesses; trim `ResolutionReport`/`MemberResolutionReport`/`ImplicitTriggerResolutionReport`/`EventFlowGateReport` to surviving cores or delete if fully subsumed (keep diagnostic fields the fixture/golden tests use; bump any externally-serialized schema). Delete the matching CDO-gated L3-comparison test fns.
- [ ] **Step 3: Run** `cargo test --workspace` (no CDO) — green on the surviving contracts + frozen goldens + fixtures. Grep-confirm `src/program/resolve` has zero `engine::l3`/`engine::l2` imports EXCEPT `builtins.rs::global_builtins` (document the exception in a comment).
- [ ] **Step 4: rustfmt + clippy + (CDO) the frozen audits green + commit** — `refactor(resolve): remove L3 oracle (project_l3*) from the gates; engine self-validated (1B.3b Task 3)`.

---

### Task 4: Verify the engine stands alone + honest CHANGELOG

**Files:** Modify `CHANGELOG.md`, the charter memory note; Test (final verification).

- [ ] **Step 1: Verify** — `cargo test --workspace` (NO `CDO_WS`) FULLY GREEN (the engine validates WITHOUT the oracle); clippy clean; grep confirms `src/program/resolve` (except `builtins.rs`) is `engine::l3`-free. (WITH `CDO_WS` + `ENFORCE_CDO_WS=1`) the frozen audits (all dispatch kinds) + `route_applicability` + the Histogram ceiling green, deterministic, `checked_sites>0`.
- [ ] **Step 2: CHANGELOG** (honest, per gpt #5): 1B.3b retires the L3 oracle from the fresh resolver's VALIDATION. The engine is now validated by: (a) committed ANONYMIZED frozen L3-verdict goldens (Member/Interface + ImplicitTrigger + EventFlow — per-site target identity, the CDO-scale floor active on the gated/internal runner that has the CDO source; public CI validates their metadata); (b) the L3-INDEPENDENT contracts (`coverage_holds`, `evidence_overclaim`, `abi_unmapped`, `route_applicability` with the ported fan-out teeth as SOUNDNESS checks, the Histogram + real-unknown ceiling); (c) always-run synthetic semantic fixtures (incl. the converted EventFlow + the new ImplicitTrigger target-set fixtures). State plainly: this is NOT first-principles semantic correctness — it is the frozen historical L3 verdict + contracts + fixtures; the teeth are soundness, the frozen goldens carry completeness. L3-minting moved to a dev tool; `src/engine/l3` stays (aldump backbone); `builtins.rs::global_builtins` data dependency stays. Out-of-scope deferrals stated (genuine_wrong disambig, fresh⊆l3 recall, Cat-D dispatch). The fresh engine now STANDS ALONE.
- [ ] **Step 3: Commit** — `docs(resolve): 1B.3b complete — fresh engine self-validated, L3 oracle retired from gates (1B.3b Task 4)`.

---

## Roadmap — beyond 1B.3b (the engine stands alone)

genuine_wrong=42 disambiguation (mostly L3-error-on-builtins — confirm+fix); `fresh⊆l3` partial-recall full validation; same-arity-type overload DISPATCH; the snapshot-composition root cause; table/page/database trigger-events as EventFlow; BindSubscription activation; the receiver-gap buckets. North-star: drive the self-reported real-unknown rate toward its provably-dynamic residual.

## Self-Review

- **Round-1 reviewer fixes incorporated:** (1) ANONYMIZED committed per-site CDO golden (all dispatch kinds) — not digest-only — so the CDO-scale floor is a runnable reviewable artifact (gated runner); (2) ImplicitTrigger + EventFlow get EXTENDED frozen TARGET-SET baselines (option a) + kept/converted fixtures — not delete-and-rely-on-teeth; the teeth are SOUNDNESS-only (completeness from the goldens/fixtures); (3) `ENFORCE_CDO_WS` hard-fail + public-CI metadata validation — no fail-open silent skip; (4) L3-minting moved to a dev-only tool — the gate module is L3-free; (5) port-and-prove teeth (Task 2) BEFORE deleting gates (Task 3); (6) honest CHANGELOG (frozen historical verdict + contracts, not first-principles).
- **The spine (freeze-and-prove-before-remove)** is the task order; the biggest risk (remove/delete before the frozen+ported replacement is green) is structurally prevented.
- **Spec coverage:** §6 the gate becomes frozen goldens + L3-independent contracts (Tasks 1-2); removal (Task 3); verification (Task 4).
- **Placeholder scan:** "mirror run_semantic_diff / capture PRecordOperation+project_event_graph / port the four predicates / stable-hash anon" name exact sources. No `TODO`.
- **Type consistency:** `anon_id`/the anonymized goldens (Task 1) → the audits + `route_applicability` (Tasks 1-2); the ported teeth (Task 2) consume `applicability.rs` + `verify_event_subscriber_route`.
