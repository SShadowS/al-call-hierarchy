# Plan 1B.2 Phase 2 — Core Resolution + Global-Builtin Catalog Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Emit the first REAL resolved routes — resolve `Bare` and `ObjectRun` call sites topology-scoped via `ResolveIndex`, plus implicit-trigger edges, each carrying real `Evidence` (Source/Catalog/Abi/Unknown) + a `Witness` — and prove via the dual-run gate that fresh resolution matches-or-beats L3 on those sites (zero REGRESSION / EVIDENCE_OVERCLAIM, in-scope real-unknown ≤ L3).

**Architecture:** Builds on Phase 1's `src/program/resolve/` extraction + indexes. First fixes two foundational modelling gaps (snapshot self-dependency pollution; `RoutineNodeId` trigger discriminator) the Phase-1 review flagged. Then a clean-room global-builtin catalog module (its own disposition→evidence logic; membership data from the in-repo `tools/gen-al-builtins` generator), a `Bare`/`ObjectRun` resolver mirroring L3's precedence over `ResolveIndex`/`BodyMap`, implicit-trigger edges for record-ops, and the Phase-2 resolution gate.

**Tech Stack:** Rust (edition 2024, toolchain 1.96.0), the `al-call-hierarchy` crate, the `src/engine/l3` resolver as the read-only ORACLE.

**Source of truth:** `docs/superpowers/specs/2026-06-29-plan1b2-fresh-resolver-design.md` (§5.3 resolution, §5.4 evidence, §5.5 witness, §5.6 catalog, §6 gate). Read it first.

## Key facts grounding this plan (from the Phase-2 L3-resolution map)

- **L3 Bare precedence** (`call_resolver.rs:465-616`): own-object (`Direct/Resolved`) → extension base-object (via `extends_base_object`) → implicit-Rec table + its TableExtensions → global-builtin (`global_builtin_disposition(name).is_some()` → `Builtin/Builtin`) → else `Unresolved/Unknown(BareUnresolved)`.
- **L3 ObjectRun** (`call_resolver.rs:618-669`): no target → `Dynamic/Unknown(DynamicObjectRunTarget)`; target by name (`object_by_type_name`) or number (`object_by_type_number`); not found in source → `Opaque`; found → entry trigger (`routine_in_object(id,"OnRun")` else first routine) → `Resolved`. **L3 always uses `"OnRun"` even for Page/Report — Phase 2 corrects to `OnOpenPage` (Page) / `OnPreReport` (Report).**
- **Builtin catalog generator EXISTS:** `tools/gen-al-builtins/gen.csproj` (.NET) extracts 785 names from `Microsoft.Dynamics.Nav.CodeAnalysis.dll`. `global_builtins.rs` is `@generated`; `al_builtins.rs::global_builtin_disposition` is the Tier-1 hand overlay (`"error"`→`"control-terminating"`, ~23 → `"pure-terminal"`, else generated set → `"pure-terminal"`).
- **TrustTier** (`identity.rs:21`): Workspace/EmbeddedSource/LocalSourceVerified/LocalSourceApproximate/SymbolOnly. Source-tiers → `Evidence::Source` + `RouteTarget::Routine`; SymbolOnly public proc → `Evidence::Abi` + `RouteTarget::AbiSymbol{app,symbol_key}` (deps NOT in ProgramGraph — reached via `AppUnit.abi: Option<ParsedAppPackage>`); builtin → `Evidence::Catalog` + `RouteTarget::Builtin`.
- **Implicit-trigger edges** (`implicit_edges.rs`): L3 maps record-op `Validate→OnValidate(Resolved)`, `Insert/Modify/Delete→On*(Maybe)`, keyed by `op.id`, anchored at the record-op position.
- **Prereq B fix:** `snapshot.rs:~159` — skip a dep whose identity matches `workspace_app` (the workspace's own `.app` cached in `.alpackages` interns to the same `AppRef` → duplicate nodes).
- **Prereq C fix:** add `enclosing_member_lc: Option<String>` to `RoutineNodeId` (`node.rs:86`); populate at 3 production sites (`node_extract.rs:73`, `stub.rs:57`, `body_map.rs:48`) from `RoutineDecl.enclosing_member`; 7 test sites use `None`.

## Global Constraints

- Rust edition 2024; toolchain 1.96.0. `rustfmt <file>` per-file — **never** `cargo fmt`.
- Stage only files each task names — **never** `git add -A`.
- CI gates: `cargo clippy --release --all-features -- -D warnings` (NO `--tests`), `cargo fmt --check`, `cargo test --workspace`.
- **`src/engine/l3` + goldens are the read-only ORACLE — do not modify.** Do NOT import L3's `al_builtins.rs` disposition logic into the fresh catalog (clean-room: fresh owns its disposition→evidence; membership data may come from the generator's authoritative output).
- Determinism: emitted collections sorted; harness run-to-run identical.
- CDO env-gated: `CDO_WS` (= `U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud`); tests return early when unset — run WITH it set.
- Update `CHANGELOG.md` per user-visible capability.
- **rust-analyzer diagnostics on `resolve/*` are NOT authoritative** (confirmed ~17×, incl. E0716/E0432 compile-error-class) — only `cargo build`/`test`/`clippy` count.

## File / module structure (this phase)

| File | Responsibility |
|------|----------------|
| `src/snapshot/snapshot.rs` (modify) | Prereq B: identity-based self-dependency exclusion. |
| `src/program/node.rs` (modify) | Prereq C: `RoutineNodeId.enclosing_member_lc`. |
| `src/program/{node_extract.rs, resolve/stub.rs, resolve/body_map.rs}` (modify) | Prereq C: populate the new field. |
| `src/program/resolve/builtins.rs` (create) | Clean-room global-builtin catalog: membership + fresh disposition→evidence + coverage contract. |
| `src/program/resolve/resolver.rs` (create) | `resolve_bare` / `resolve_object_run` / `resolve_implicit_trigger` over `ResolveIndex`/`BodyMap`; evidence + witness assignment. |
| `src/program/resolve/differential.rs` (modify) | Wire real routes into the gate; resolved-route diff vs L3 for Bare/Run; new gate buckets. |
| `tests/program_resolve_harness.rs` (modify) | Phase-2 resolution gate (env-gated CDO). |

---

### Task 1: Prereq B — snapshot self-dependency exclusion

**Files:**
- Modify: `src/snapshot/snapshot.rs`
- Test: `src/snapshot/snapshot.rs` (`#[cfg(test)]`, env-gated) + `src/program/build.rs` test if a unit test fits

**Interfaces:**
- Consumes: `AppId`, `workspace_app`, the dep loop in `SnapshotBuilder::build`.
- Produces: no API change — a behavioral fix (the workspace's own `.app` is no longer added as a dep `AppUnit`).

- [ ] **Step 1: Write the failing test** — env-gated CDO test asserting the workspace app identity appears EXACTLY ONCE in `snap.apps` (no self-dep duplicate), and that `build_program_graph(&snap)` has no duplicate `RoutineNodeId` for the workspace app:

```rust
#[test]
fn workspace_is_not_its_own_dependency() {
    let Some(ws) = std::env::var_os("CDO_WS").map(std::path::PathBuf::from).filter(|p| p.exists()) else { return; };
    let snap = crate::snapshot::SnapshotBuilder { workspace_root: ws, local_providers: vec![] }.build().expect("snapshot");
    // The workspace app identity must appear exactly once across all units.
    let ws_id = &snap.workspace_app;
    let same = snap.apps.iter().filter(|u| u.id.guid == ws_id.guid && !ws_id.guid.is_empty()).count();
    assert_eq!(same, 1, "workspace .app cached in .alpackages must not be added as a self-dependency");
}
```

- [ ] **Step 2: Run to verify it fails** — `CDO_WS=... cargo test -p al-call-hierarchy snapshot::snapshot::tests::workspace_is_not_its_own_dependency -- --nocapture`. Expected: FAIL (count == 2) if the ancestor `.alpackages` holds the workspace `.app`; if the fixture doesn't trigger it, the test passes trivially — note that and keep it as a guard.
- [ ] **Step 3: Implement the exclusion** — in `SnapshotBuilder::build`, after `dep_id` is fully built (around the dep loop, ~line 159) and BEFORE the `AppUnit` is constructed, `continue` when the dep identity matches `workspace_app`:

```rust
// Self-dependency guard: the workspace's own compiled .app can sit in an ancestor
// `.alpackages` (monorepo / CI cache). It interns to the SAME AppRef as the workspace
// source, polluting the graph with duplicate nodes. Exclude it.
if (!workspace_app.guid.is_empty() && dep_id.guid == workspace_app.guid)
    || (workspace_app.guid.is_empty()
        && dep_id.name.eq_ignore_ascii_case(&workspace_app.name)
        && dep_id.version == workspace_app.version)
{
    continue;
}
```

- [ ] **Step 4: Run to verify it passes** — the env-gated test now asserts count == 1; also run the existing `builds_snapshot_over_cdo_workspace` (it asserts `apps.len() >= 10` — confirm still holds after dropping one self-dep).
- [ ] **Step 5: Format, lint, full gate, commit**

```bash
rustfmt src/snapshot/snapshot.rs
cargo clippy --release --all-features -- -D warnings 2>&1 | tail -3
cargo test --workspace 2>&1 | grep -E "test result:|FAILED" | tail
git add src/snapshot/snapshot.rs
git commit -m "fix(snapshot): exclude workspace's own .app from its dependency closure (Phase 2 prereq B)"
```

---

### Task 2: Prereq C — `RoutineNodeId` trigger discriminator

**Files:**
- Modify: `src/program/node.rs`, `src/program/node_extract.rs`, `src/program/resolve/stub.rs`, `src/program/resolve/body_map.rs`
- Test: `src/program/resolve/body_map.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces: `RoutineNodeId { object: ObjectNodeId, name_lc: String, enclosing_member_lc: Option<String> }`. Every consumer that constructs a `RoutineNodeId` gains the field.

- [ ] **Step 1: Write the failing test** — a `body_map` test with a table-extension declaring TWO same-named field triggers (`OnValidate` on two different fields) asserting `BodyMap::get` returns DISTINCT routines for each (no last-wins collision):

```rust
#[test]
fn same_named_field_triggers_are_distinct() {
    let src = r#"
tableextension 50100 "Cust Ext" extends Customer
{
    fields
    {
        field(50100; Foo; Integer) { trigger OnValidate() begin Bar(); end; }
        field(50101; Baz; Integer) { trigger OnValidate() begin Qux(); end; }
    }
}
"#;
    let file = al_syntax::parse(src);
    // Build a 1-unit ProgramGraph + BodyMap (mirror the existing body_map test construction).
    // Assert the two OnValidate routines have DISTINCT RoutineNodeIds (different enclosing_member_lc: "foo" vs "baz")
    // and BodyMap::get returns each (no collision).
    // (Construct the graph/parsed unit as the existing body_map tests do.)
}
```

> Confirm the IR exposes `enclosing_member` on the two triggers (parse the fixture, inspect). If al_syntax doesn't populate `enclosing_member` for table-extension field triggers, fall back to asserting via the `node_extract` path. The KEY assertion: the two routines are not the same `RoutineNodeId`.

- [ ] **Step 2: Run to verify it fails** — currently both triggers key to `(object, "onvalidate", —)` → last-wins → the test fails (only one retrievable / they're equal).
- [ ] **Step 3: Implement** — add `pub enclosing_member_lc: Option<String>` to `RoutineNodeId` (`node.rs:86`, keep all derives — `Hash/Eq/Ord` auto-extend). Populate at the 3 production sites from `r.enclosing_member.as_ref().map(|(n,_)| n.to_ascii_lowercase())`: `node_extract.rs:73`, `stub.rs:57`, `body_map.rs:48`. Set `None` at the 7 test construction sites (`stub.rs` synthetic helper, `body_map.rs` tests ×4, `edge.rs` `rid()`, `index.rs` `make_routine()`/`fake_pub`). Build the whole workspace to find every site (the compiler errors list them).
- [ ] **Step 4: Run to verify it passes** — the new test + all existing `program::resolve::` tests.
- [ ] **Step 5: Format, lint, full gate, commit**

```bash
rustfmt src/program/node.rs src/program/node_extract.rs src/program/resolve/stub.rs src/program/resolve/body_map.rs
cargo clippy --release --all-features -- -D warnings 2>&1 | tail -3
cargo test --workspace 2>&1 | grep -E "test result:|FAILED" | tail
git add src/program/node.rs src/program/node_extract.rs src/program/resolve/stub.rs src/program/resolve/body_map.rs
git commit -m "feat(program): RoutineNodeId enclosing-member discriminator for same-named triggers (Phase 2 prereq C)"
```

---

### Task 3: Clean-room global-builtin catalog

**Files:**
- Create: `src/program/resolve/builtins.rs`
- Modify: `src/program/resolve/mod.rs`
- Test: `src/program/resolve/builtins.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces: `pub fn is_global_builtin(name_lc: &str) -> bool`; `pub fn global_builtin_id(name_lc: &str) -> Option<BuiltinId>` (returns a `BuiltinId` for a recognized builtin); `pub fn catalog_version() -> &'static str` (the AL-ext provenance string, for `Witness::CatalogEntry`).

- [ ] **Step 1: Write the failing coverage-contract test** — assert the fresh catalog recognizes EVERY name L3's catalog does (coverage parity; a builtin L3 knows that fresh misses → a real-unknown the gate would surface):

```rust
#[test]
fn fresh_catalog_covers_l3_catalog() {
    // Every name L3 recognizes as a global builtin, fresh must too (coverage parity).
    // Sample the L3 catalog via its public predicate over a representative name set
    // AND assert the union size matches the documented 785 (provenance guard).
    for n in ["error","message","confirm","format","strlen","today","createguid","abs","round","strsubstno"] {
        assert!(is_global_builtin(n), "fresh catalog must recognize builtin {n}");
        assert!(global_builtin_id(n).is_some());
    }
    assert!(!is_global_builtin("definitely_not_a_builtin_xyz"));
    assert!(!catalog_version().is_empty());
}
```

- [ ] **Step 2: Run to verify it fails.**
- [ ] **Step 3: Implement `builtins.rs`** — the fresh catalog. **Membership data:** the authoritative AL-compiler global-builtin set (regenerate-able via `dotnet run --project tools/gen-al-builtins/gen.csproj`; document this + the AL-ext version as `catalog_version()`). To avoid a dotnet dependency in CI, reference the generator's authoritative output (the 785-name set) — but write FRESH disposition→evidence logic in this module (do NOT import `al_builtins.rs`). `global_builtin_id` returns `BuiltinId(name_lc.to_string())`. Add a contract test that the membership is non-trivial (≥700 names) so a broken regen is caught.

> Clean-room note: the membership set is platform truth (the AL compiler's intrinsic list), not L3 logic — sourcing it from the generator's output is authoritative, not "seeding from L3". The disposition→evidence mapping (what this module does with a hit) is written fresh here, not copied from `al_builtins.rs`.

- [ ] **Step 4: Run to verify it passes.**
- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/program/resolve/builtins.rs src/program/resolve/mod.rs
cargo clippy --release --all-features -- -D warnings 2>&1 | tail -3
git add src/program/resolve/builtins.rs src/program/resolve/mod.rs
git commit -m "feat(resolve): clean-room global-builtin catalog with coverage contract (Phase 2 Task 3)"
```

---

### Task 4: `Bare`-call resolution with evidence + witness

**Files:**
- Create: `src/program/resolve/resolver.rs`
- Modify: `src/program/resolve/mod.rs`
- Test: `src/program/resolve/resolver.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `ResolveIndex` (`routines_in_object`, `object_by_number`, `table_extensions_of`), `BodyMap`, `is_global_builtin`/`global_builtin_id`/`catalog_version`, `RawSiteV2`/`CalleeShape::Bare`, the `Edge`/`Route`/`Evidence`/`Witness`/`RouteTarget` types, `ObjectNode.tier`/`RoutineNode.tier`.
- Produces: `pub fn resolve_bare(from_object: &ObjectNodeId, name_lc: &str, arity: usize, graph: &ProgramGraph, index: &ResolveIndex) -> Vec<Route>` returning the resolved route(s):
  - own-object: `routines_in_object(from_object, name_lc)` (arity-filter) → `Route{ Routine(id), evidence: tier_evidence(target.tier), witness: SourceSpan{...}, .. }`.
  - else extension base: if `from_object` is a `*Extension`, resolve its base (via `ObjectNode.extends_target` → `resolve_object(from.app, base_kind, target)`) and look up there.
  - else implicit-Rec: if the object is a Page/Table(Ext) with an implicit table, look up in the table + its `table_extensions_of`.
  - else global-builtin: `is_global_builtin(name_lc)` → `Route{ Builtin(global_builtin_id), evidence: Catalog, witness: CatalogEntry{ id, catalog_version() } }`.
  - else → `Route{ Unresolved, evidence: Unknown, witness: None }`.
  - `tier_evidence(tier)`: source-tiers → `Source`; `SymbolOnly` → `Abi` (deps aren't in the graph, so own-object source-tier targets are always `Source` here; `Abi` arises only for cross-app, light in Phase 2).

- [ ] **Step 1: Write failing unit tests** — synthetic `ProgramGraph` + `ResolveIndex`: (a) a bare call to an own-object procedure → `Route{Routine, Source}`; (b) a bare `Message` → `Route{Builtin, Catalog}` with a `CatalogEntry` witness; (c) a bare call to a nonexistent non-builtin → `Route{Unresolved, Unknown}`; (d) an extension calling a base-object proc → resolved via the base. Assert evidence + witness per route. Witness↔evidence contract (Source⇒SourceSpan, Catalog⇒CatalogEntry).
- [ ] **Step 2: Run to verify they fail.**
- [ ] **Step 3: Implement `resolve_bare`** mirroring the L3 precedence (own → extension base → implicit-Rec → builtin → unknown). Use `BodyMap` to get the target routine's span for the `SourceSpan` witness (file + byte range from `RoutineDecl.origin`).
- [ ] **Step 4: Run to verify they pass.**
- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/program/resolve/resolver.rs src/program/resolve/mod.rs
cargo clippy --release --all-features -- -D warnings 2>&1 | tail -3
git add src/program/resolve/resolver.rs src/program/resolve/mod.rs
git commit -m "feat(resolve): Bare-call resolution with evidence + witness (Phase 2 Task 4)"
```

---

### Task 5: `ObjectRun` + implicit-trigger resolution

**Files:**
- Modify: `src/program/resolve/resolver.rs`
- Test: `src/program/resolve/resolver.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces:
  - `pub fn resolve_object_run(from: AppRef, object_kind: &str, target_ref: Option<&str>, graph, index) -> (DispatchShape, Vec<Route>)`: no target → `(DynamicOpen, [Unresolved/Unknown])` with `completeness: Partial{RuntimeTypeUnbounded}`; target by name/number → `resolve_object`/`object_by_number`; not in graph → `Route{ Unresolved/AbiSymbol, Opaque }`; found → entry-trigger route (`OnRun` for Codeunit, `OnOpenPage` for Page, `OnPreReport` for Report — look these up via `routines_in_object`) → `(Exact, [Routine(entry), Source])`.
  - `pub fn resolve_implicit_trigger(record_op: &RawSiteV2 /*RecordOp shape*/, table: &ObjectNodeId, graph, index) -> Vec<Edge>`: for op `Validate`→`OnValidate` (the field's trigger), `Insert`→`OnInsert`, `Modify`→`OnModify`, `Delete`→`OnDelete`; route to the table's (+ its extensions') trigger routine; `EdgeKind::ImplicitTrigger`; `Multicast` over base + extension triggers with `completeness: Partial{ReverseDependentExtensions}`; conditional (`Insert(var)` → `Condition::RunTriggerGuarded`).

- [ ] **Step 1: Write failing unit tests** — (a) `Codeunit.Run` to a known codeunit → entry `OnRun` route, `Exact`, `Source`; (b) `Page.RunModal` to a page → `OnOpenPage` route (NOT `OnRun` — the Phase-2 correction); (c) `Codeunit.Run` with no static target → `DynamicOpen` + open-world blocker; (d) a `Validate` record-op on a table with an `OnValidate` field trigger → an `ImplicitTrigger` edge to it (Multicast). Assert shapes + evidence.
- [ ] **Step 2: Run to verify they fail.**
- [ ] **Step 3: Implement.** Entry-trigger names per object kind. Implicit-trigger fan-out over `table_extensions_of`.
- [ ] **Step 4: Run to verify they pass.**
- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/program/resolve/resolver.rs
cargo clippy --release --all-features -- -D warnings 2>&1 | tail -3
git add src/program/resolve/resolver.rs
git commit -m "feat(resolve): ObjectRun + implicit-trigger resolution (Phase 2 Task 5)"
```

---

### Task 6: Phase-2 resolution gate (dual-run vs L3)

**Files:**
- Modify: `src/program/resolve/differential.rs`, `src/program/resolve/stub.rs` (the resolver now produces real routes; rename/supersede the stub), `tests/program_resolve_harness.rs`, `CHANGELOG.md`
- Test: `tests/program_resolve_harness.rs` (env-gated CDO)

**Interfaces:**
- Consumes: `resolve_bare`/`resolve_object_run`/`resolve_implicit_trigger` (Tasks 4-5), `project_l3` (CallEdge oracle, Phase 0), the `match_sites` matcher, the gate buckets.
- Produces: wire real routes into the fresh canonical projection (a resolved `Bare`/`Run` site now has NON-empty `targets`); compare vs L3's `CallEdge` targets for the same sites. Add the spec §6.3 buckets: `REGRESSION` (L3 resolved, fresh Unknown), `VERIFIED_WIN` (L3 Unknown, fresh resolved + witnessed), `UNVERIFIED_EXTRA`, `EVIDENCE_OVERCLAIM` (fresh claims Source without a span witness, etc.), `DIVERGENCE`. Compute the in-scope (`Bare`+`Run`) `real_unknown_rate`.

- [ ] **Step 1: Write the failing CDO gate test**

```rust
#[test]
fn phase2_bare_run_resolution_matches_or_beats_l3() {
    let Some(ws) = std::env::var_os("CDO_WS").map(std::path::PathBuf::from).filter(|p| p.exists()) else { return; };
    let report = run_resolution_harness(&ws); // Bare+Run in-scope
    assert_eq!(report.regression, 0, "fresh must not lose a Bare/Run target L3 resolved: {report:?}");
    assert_eq!(report.evidence_overclaim, 0, "no Source/Abi/Catalog claim without a valid witness: {report:?}");
    assert_eq!(report.unverified_extra, 0, "no unwitnessed new edge on a non-dynamic shape: {report:?}");
    assert!(report.fresh_real_unknown_rate <= report.l3_real_unknown_rate + 1e-9,
        "in-scope real-unknown must be <= L3: {report:?}");
    assert_eq!(report, run_resolution_harness(&ws), "deterministic");
}
```

- [ ] **Step 2: Run to verify it fails.**
- [ ] **Step 3: Implement** the resolved-route projection + the bucketed diff + the in-scope real-unknown computation. Investigate any `REGRESSION`/`EVIDENCE_OVERCLAIM`/`UNVERIFIED_EXTRA` > 0 — they mean a resolution divergence to fix (a precedence gap, a wrong evidence/witness, or an over-approximation), NOT a threshold to relax. A `VERIFIED_WIN` (fresh resolves where L3 was Unknown — e.g. a builtin L3's source-only path missed) is logged, allowed.
- [ ] **Step 4: Run with CDO_WS set** — print the breakdown; confirm body ran; all asserts pass. Investigate before adjusting anything.
- [ ] **Step 5: Full gate + commit**

```bash
rustfmt src/program/resolve/differential.rs src/program/resolve/stub.rs tests/program_resolve_harness.rs
cargo clippy --release --all-features -- -D warnings 2>&1 | tail -3
cargo test --workspace 2>&1 | grep -E "test result:|FAILED" | tail
git add src/program/resolve/differential.rs src/program/resolve/stub.rs tests/program_resolve_harness.rs CHANGELOG.md
git commit -m "feat(resolve): Phase-2 Bare/Run resolution gate vs L3 (Phase 2 Task 6)"
```

---

## Roadmap — Phase 3+ (next plans)

Phase 3: clean-room receiver-type lattice + member-builtin catalog (resolve `Member` sites — Record/RecordRef/FieldRef/CurrPage/framework). Phase 4: Polymorphic (interface/enum) + Multicast (event/extension) fan-out + open-world completeness. 1B.3: full SymbolReference ABI cross-check (verify Abi routes), deep re-baseline, retire L3 oracle. Phase-2 deferral: real `has_implicit_rec` predicate (codeunit-no-TableNo / page-no-source / proc-name-collision) plumbed into `extract.rs` — the Phase-1 `dataitem_source_table` proxy is broader than Reports.

## Self-Review

- **Spec coverage:** §5.3 Bare/ObjectRun/implicit-trigger → Tasks 4-5; §5.4 evidence (tier→Evidence) → Task 4; §5.5 witness → Tasks 4-6 (EVIDENCE_OVERCLAIM gate); §5.6 catalog → Task 3; §6 gate → Task 6; the two Phase-1-review prereqs → Tasks 1-2. Member resolution (§5.2) correctly deferred to Phase 3. Abi-evidence cross-app routes are modeled (RouteTarget::AbiSymbol) but light in Phase 2 (deps not in graph; full ABI lookup is 1B.3).
- **Placeholder scan:** the "confirm IR exposes enclosing_member / mirror L3 precedence / regenerate via dotnet" steps are bounded verification naming the exact file + action. No `TODO`/`add appropriate X`.
- **Type consistency:** `RoutineNodeId.enclosing_member_lc` (Task 2) threads through all consumers; `is_global_builtin`/`global_builtin_id`/`catalog_version` (Task 3) → Task 4; `resolve_bare`/`resolve_object_run`/`resolve_implicit_trigger` (Tasks 4-5) → Task 6; `Evidence`/`Witness`/`RouteTarget`/`DispatchShape`/`Edge` reused from the Phase-0 edge model.
- **Known follow-ups:** the real `has_implicit_rec` predicate (Phase 3); full ABI-symbol resolution for cross-app Abi routes (1B.3); `catalog_version` regen on AL-ext bumps.
