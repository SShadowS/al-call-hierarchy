# Plan 1B.2 Phase 1 — Structured Extraction + Resolve Indexes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Phase 0's minimal call-expression walk with STRUCTURED, classified call-site extraction over the al-syntax IR, build the topology-scoped resolve indexes + body map, and prove (via the dual-run harness) that fresh's call-site classification reconciles with L3's PCallSite model — so resolution (Phases 2–4) builds on a verified extraction layer.

**Architecture:** Extends `src/program/resolve/` (merged in Phase 0). Adds structured extraction (`extract.rs`) classifying each `ExprKind::Call` into a `CalleeShape` mirroring L3's `classify_callee` + record-op filter (the key to reconciling the 58k Phase-0 extra_site), a `ResolveIndex` (the lookups Phase 2 resolution needs, each WorldMode-tagged), a `BodyMap` (NodeId → routine decl), and an L3-PCallSite projection so the harness can diff at SITE granularity and categorize the record-op/commit extras.

**Tech Stack:** Rust (edition 2024, toolchain 1.96.0), the `al-call-hierarchy` crate, `al_syntax` IR, the existing `src/engine/l3` resolver as the read-only oracle.

**Source of truth:** `docs/superpowers/specs/2026-06-29-plan1b2-fresh-resolver-design.md` (§5.1 extraction, §4.1 ResolveIndex+WorldMode). Read it before starting.

## Key facts grounding this plan (from the L3 call-site model map)

- L3 emits a `PCallSite` for **every `ExprKind::Call`/`StmtKind::Call` EXCEPT** (a) **record ops** — a call whose receiver is a record-typed variable (`rvars`: record locals/params/globals + always `rec`/`xrec`) AND whose method is one of 28 record-op names (`record_op.rs`), and (b) bare `Commit()`. These go to record-operations/operation-sites instead. (`Error()` is BOTH an op-site and a PCallSite.)
- `resolve_calls` has NO early-skip: every PCallSite → ≥1 `CallEdge` (interface dispatch is the only 1→N). So **L3 PCallSite count ≈ L3 edge count** (minus interface fan-out, plus implicit-trigger edges).
- Record ops `Validate`/`Insert`/`Modify`/`Delete` (with a resolved table) additionally produce `DispatchKind::ImplicitTrigger` edges keyed by `op.id` (`{routine}/op{N}`), anchored at the record-op call position — these are data-is-control-flow edges with NO PCallSite.
- Event publisher→subscriber edges are SEPARATE (`event_graph.rs`, not in `resolve_calls`).
- `classify_callee` (`ir_walk.rs`): `Identifier`/`QuotedIdentifier`→Bare; `Member` on a `keyword_identifier` receiver (`codeunit`/`page`/`report`) with method `run`→ObjectRun; other `Member`→Member; else Unknown.
- The 58,256 Phase-0 extra_site are predominantly record-op + commit sites that fresh counts but L3 doesn't make PCallSites for. **They are justified-extra.**

## Global Constraints

- Rust edition 2024; toolchain pinned 1.96.0. Format per-file with `rustfmt <file>` — **never** `cargo fmt`.
- Stage only the files each task names — **never** `git add -A` / `git add .`.
- CI gates every commit: `cargo clippy --release --all-features -- -D warnings` (NO `--tests` — that surfaces unrelated pre-existing lint debt), `cargo fmt --check`, `cargo test --workspace`. All must pass.
- **The `src/engine/l3` resolver + its goldens are the read-only ORACLE — do not modify them.**
- Determinism: all emitted collections sorted by a stable key; harness runs byte-identical run-to-run.
- CDO fixture env-gated: tests needing it read `CDO_WS` (= `U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud`) and **return early when unset** — run them WITH the env set to genuinely assert.
- Update `CHANGELOG.md` under `## [Unreleased]` → `### Added`/`### Changed` once per task that adds a user-visible capability.
- New `resolve/*` code folds names with `to_ascii_lowercase` (matches existing `src/program/`).
- **rust-analyzer diagnostics on `resolve/*` are NOT authoritative** (confirmed ~16× on the Phase-0 branch, incl. E0255/E0432-class) — only `cargo build`/`test`/`clippy` results count.

## File / module structure (this phase)

| File | Responsibility |
|------|----------------|
| `src/program/resolve/differential.rs` (modify) | Prereqs (object_kind parity test, regression classifier); add `project_l3_sites` (PCallSite projection) + the categorized site-level gate fields. |
| `src/program/resolve/extract.rs` (create) | Structured extraction: `CalleeShape`, `SiteKind`, `RawSiteV2`, `extract_sites(routine, src, unit, rvars) -> Vec<RawSiteV2>`; `rvars` detection from a routine's params/locals/globals. |
| `src/program/resolve/body_map.rs` (create) | `BodyMap`: `NodeId -> &RoutineDecl` over the parsed snapshot. |
| `src/program/resolve/index.rs` (create) | `ResolveIndex` + `WorldMode`: routine-overload, object-by-number, table, table-extension-by-base, interface/enum-implementer, event-subscriber lookups. |
| `tests/program_resolve_harness.rs` (modify) | Phase-1 site-parity gate (env-gated CDO) + unit fixtures. |

---

### Task 1: Phase-0 prereqs — object_kind parity test + regression classifier

**Files:**
- Modify: `src/program/resolve/differential.rs`
- Test: `src/program/resolve/differential.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `make_canonical_key`, `object_kind_str` (Phase 0), `crate::program::node::ObjectKind`.
- Produces: a contract test `from_side_object_kind_parity`; the regression-classifier change in `run_harness`.

- [ ] **Step 1: Write the failing test** (parity of the from-side kind string between fresh `format!("{k:?}").to_ascii_lowercase()` and the L3 `object_type` lowercased, for every `ObjectKind`)

```rust
#[test]
fn from_side_object_kind_parity() {
    // Fresh derives the caller key's object_kind via Debug-lowercase; L3 derives it from
    // its `object_type` string lowercased. They MUST agree for every kind or sites silently
    // drop out of `matched`. This asserts the canonical spelling for each variant.
    use crate::program::node::ObjectKind;
    let cases = [
        (ObjectKind::Codeunit, "codeunit"),
        (ObjectKind::Table, "table"),
        (ObjectKind::TableExtension, "tableextension"),
        (ObjectKind::Page, "page"),
        (ObjectKind::PageExtension, "pageextension"),
        (ObjectKind::Report, "report"),
        (ObjectKind::ReportExtension, "reportextension"),
        (ObjectKind::XmlPort, "xmlport"),
        (ObjectKind::Query, "query"),
        (ObjectKind::Enum, "enum"),
        (ObjectKind::EnumExtension, "enumextension"),
        (ObjectKind::Interface, "interface"),
        (ObjectKind::ControlAddIn, "controladdin"),
        (ObjectKind::PermissionSet, "permissionset"),
        (ObjectKind::PermissionSetExtension, "permissionsetextension"),
        (ObjectKind::Profile, "profile"),
        (ObjectKind::Entitlement, "entitlement"),
    ];
    for (k, expected) in cases {
        assert_eq!(format!("{k:?}").to_ascii_lowercase(), expected, "kind {k:?}");
    }
}
```

> Confirm the exact `ObjectKind` variant set in `src/program/node.rs` / `crates/al-syntax/src/ir/decl.rs` and cover EVERY variant. If a variant's Debug-lowercase does NOT match the L3 `object_type` spelling for that kind (read `ir_object_type` in `src/engine/l2/ir_walk.rs`), that is a real bug — fix the canonical encoding (route the from-key through the shared `object_kind_str_to_tag`/string helper for both sides) rather than weakening the test.

- [ ] **Step 2: Run test to verify it fails or passes** — `cargo test -p al-call-hierarchy program::resolve::differential::tests::from_side_object_kind_parity`. If it PASSES immediately (parity already holds — likely, since `missing_site=0` on CDO proves zero drift), that is acceptable: this is a regression-guard. If it FAILS for a kind, fix the encoding so both sides agree, then it passes.

- [ ] **Step 3: Fix the regression classifier** — in `run_harness`, the Paired-bucket arm currently counts `regression` when only `fresh_canonical[fi].targets.is_empty()`. Change it to require the L3 side non-empty too (track the l3 index `li` from the `Paired(fi, li)` and check `!l3_canonical[li].targets.is_empty()`):

```rust
// In run_harness, the Paired(fi, li) arm:
if fresh_canonical[*fi].targets.is_empty() && !l3_canonical[*li].targets.is_empty() {
    regression += 1;
}
```

(Read the current arm; `li` is currently `_li` — un-underscore it and use it. matched still counts all Paired.)

- [ ] **Step 4: Run the harness unit tests + (env-gated) CDO gate** — `cargo test -p al-call-hierarchy --test program_resolve_harness` (synthetic ones); with `CDO_WS` set, confirm `regression == matched` still holds (in Phase 1 the stub is unchanged so fresh targets are still empty AND L3 targets are non-empty for resolved sites → identical count).

- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/program/resolve/differential.rs
cargo clippy --release --all-features -- -D warnings 2>&1 | tail -3
git add src/program/resolve/differential.rs
git commit -m "fix(resolve): object_kind parity guard + regression classifier checks L3 side (Phase 1 Task 1)"
```

---

### Task 2: Structured call-site extraction with classification

**Files:**
- Create: `src/program/resolve/extract.rs`
- Modify: `src/program/resolve/mod.rs` (add `pub mod extract;`)
- Test: `src/program/resolve/extract.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `al_syntax::ir` (`AlFile`, `ExprKind`, `StmtKind`, `RoutineDecl`, `VarDecl`, `Param`, `Origin`); Phase-0 `CanonicalSpan`/`SourcePos` from `edge.rs`; the `byte_to_pos` helper pattern from `extract_min.rs`.
- Produces:
  - `pub enum CalleeShape { Bare { name: String }, Member { receiver_text: String, method: String }, ObjectRun { object_kind: String, target_ref: Option<String> }, RecordOp { receiver_text: String, op: String }, Commit, Unknown }`
  - `pub struct RawSiteV2 { pub caller_routine: String /*name_lc*/, pub shape: CalleeShape, pub arity: usize, pub span: CanonicalSpan }` (derive Debug, Clone, PartialEq, Eq)
  - `pub fn record_op_names() -> &'static [&'static str]` — the 28 record-op method names (lowercased), copied verbatim from `src/engine/l3/record_op.rs` (read it; list: findset, findfirst, findlast, find, get, calcfields, calcsums, testfield, modify, modifyall, insert, delete, deleteall, setloadfields, addloadfields, setrange, setfilter, setcurrentkey, reset, copy, transferfields, validate, init, next, count, countapprox, isempty, locktable).
  - `pub fn routine_rvars(routine: &RoutineDecl) -> std::collections::HashSet<String>` — lowercased names of record-typed params + locals, PLUS `"rec"` and `"xrec"` always. A var/param is record-typed when its `ty` (lowercased, trimmed) starts with `"record "` (confirm the AL type-text shape by reading `src/engine/l3/record_types.rs::is_record_type` and mirror it).
  - `pub fn extract_sites(file: &AlFile, src: &str, unit: &str, object_globals: &HashSet<String>) -> Vec<RawSiteV2>` — walks every object's routines; for each routine computes `rvars = routine_rvars(routine) ∪ object_globals(record-typed)`; classifies each `ExprKind::Call` per the table below; sorts by `(caller_routine, span.start)`.

**Classification rules (mirror L3's `classify_callee` + record-op filter):**

| Call `function` shape | Receiver in rvars? | method in record_op_names? | → `CalleeShape` |
|---|---|---|---|
| `Member{object: Identifier(r), member}` | yes | yes | `RecordOp { receiver_text: r, op: member }` |
| `Member{object: kw "codeunit"/"page"/"report", member="run"}` | — | — | `ObjectRun { object_kind, target_ref: first-arg-text }` |
| `Member{object, member}` (other) | — | — | `Member { receiver_text, method }` |
| `Identifier("commit")` (bare) | — | — | `Commit` |
| `Identifier(name)` where implicit-Rec frame & name in record_op_names | (frame is_record) | yes | `RecordOp { receiver_text: "rec", op: name }` |
| `Identifier(name)` / `QuotedIdentifier(name)` | — | — | `Bare { name }` |
| anything else | — | — | `Unknown` |

(For the implicit-Rec bare record-op case, Phase 1 MAY approximate by only handling the explicit `Member`-receiver record-op form first and treating bare record-op-named calls as `Bare` — note any such approximation; the gate will reveal residual extra. Prefer handling it if the IR makes the enclosing-member/`dataitem_source_table` context available on `RoutineDecl`.)

- [ ] **Step 1: Write the failing test** — a fixture codeunit with: a bare call `Foo()`, a member call `Helper.Process()`, a record op `Rec.SetRange(X)` (declare `Rec: Record Item;`), an object-run `Codeunit.Run(Codeunit::"Other")`, a `Commit()`, and a non-record member `Json.Add('a')` (declare `Json: JsonObject;`). Assert each site's `CalleeShape` variant + that the `Rec.SetRange` site is `RecordOp{op:"setrange"}` and `Json.Add` is `Member` (NOT RecordOp — `Json` is not in rvars). Assert total site count.

```rust
#[test]
fn classifies_call_shapes() {
    let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    var
        Rec: Record Item;
        Json: JsonObject;
    begin
        Foo();
        Helper.Process();
        Rec.SetRange(Status);
        Codeunit.Run(Codeunit::"Other");
        Json.Add('a');
        Commit();
    end;
    procedure Foo() begin end;
}
"#;
    let file = al_syntax::parse(src);
    let sites = extract_sites(&file, src, "C.al", &std::collections::HashSet::new());
    let run: Vec<_> = sites.iter().filter(|s| s.caller_routine == "run").collect();
    assert!(run.iter().any(|s| matches!(&s.shape, CalleeShape::Bare { name } if name.eq_ignore_ascii_case("foo"))));
    assert!(run.iter().any(|s| matches!(&s.shape, CalleeShape::Member { method, .. } if method.eq_ignore_ascii_case("process"))));
    assert!(run.iter().any(|s| matches!(&s.shape, CalleeShape::RecordOp { op, .. } if op.eq_ignore_ascii_case("setrange"))));
    assert!(run.iter().any(|s| matches!(&s.shape, CalleeShape::ObjectRun { .. })));
    assert!(run.iter().any(|s| matches!(&s.shape, CalleeShape::Commit)));
    // Json.Add is a Member call, NOT a RecordOp (Json is not a record).
    assert!(run.iter().any(|s| matches!(&s.shape, CalleeShape::Member { receiver_text, method } if receiver_text.eq_ignore_ascii_case("json") && method.eq_ignore_ascii_case("add"))));
    assert!(!run.iter().any(|s| matches!(&s.shape, CalleeShape::RecordOp { receiver_text, .. } if receiver_text.eq_ignore_ascii_case("json"))));
}
```

- [ ] **Step 2: Run test to verify it fails** — `cargo test -p al-call-hierarchy program::resolve::extract`. Expected: FAIL (module/fn missing).
- [ ] **Step 3: Implement `extract.rs`** — read `src/engine/l2/ir_walk.rs` (`walk_expr`/`classify_callee`/`record_op` filter), `src/engine/l3/record_op.rs`, `src/engine/l3/record_types.rs` to mirror the classification exactly; reuse the recursive IR walk pattern from `extract_min.rs` (descend args, member objects, all stmt sub-blocks). Compute `rvars` per routine.
- [ ] **Step 4: Run test to verify it passes** — `cargo test -p al-call-hierarchy program::resolve::extract`. Expected: PASS.
- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/program/resolve/extract.rs src/program/resolve/mod.rs
cargo clippy --release --all-features -- -D warnings 2>&1 | tail -3
git add src/program/resolve/extract.rs src/program/resolve/mod.rs
git commit -m "feat(resolve): structured call-site extraction with shape classification (Phase 1 Task 2)"
```

---

### Task 3: `BodyMap` + `ResolveIndex` with `WorldMode`

**Files:**
- Create: `src/program/resolve/body_map.rs`, `src/program/resolve/index.rs`
- Modify: `src/program/resolve/mod.rs`
- Test: in each new file (`#[cfg(test)]`)

**Interfaces:**
- Consumes: 1B.1 `ProgramGraph` (`apps`, `topology`, `objects`, `routines`, `obj_index`), `RoutineNodeId`/`ObjectNodeId`/`AppRef`/`ObjKey`/`ObjectKind` (`crate::program::node`), `DependencyGraph::closure`, `crate::snapshot::ParsedUnit`, `al_syntax::ir::RoutineDecl`.
- Produces:
  - `BodyMap` (`body_map.rs`): `pub struct BodyMap<'a> { map: HashMap<RoutineNodeId, &'a RoutineDecl> }` with `pub fn build(graph: &ProgramGraph, parsed: &'a [ParsedUnit]) -> BodyMap<'a>` (anchor each routine to its `RoutineNodeId` exactly as the stub does — mirror `node_extract`/`stub.rs` object-key logic) and `pub fn get(&self, id: &RoutineNodeId) -> Option<&'a RoutineDecl>`.
  - `ResolveIndex` (`index.rs`): `pub enum WorldMode { CallerClosure(AppRef), AnalyzedSnapshot }`; `pub struct ResolveIndex { /* private maps */ }` with `pub fn build(graph: &ProgramGraph) -> ResolveIndex` and these lookups, each closure/snapshot-scoped per §4.1:
    - `pub fn routines_in_object(&self, obj: &ObjectNodeId, name_lc: &str) -> &[RoutineRef]` (overloads; `RoutineRef` = index into `graph.routines` or a cloned `RoutineNodeId`).
    - `pub fn object_by_number(&self, from: AppRef, kind: ObjectKind, declared_id: i64) -> Option<&ObjectNode>` (CallerClosure-scoped).
    - `pub fn table_extensions_of(&self, base_table: &ObjectNodeId) -> &[ObjectNodeId]` (AnalyzedSnapshot).
    - `pub fn implementers_of(&self, interface_lc: &str) -> &[ObjectNodeId]` (AnalyzedSnapshot; codeunit + enum implementers from `ObjectNode.implements`).
    - (Event-subscriber index may be a stub returning empty in Phase 1 — note it; full event modelling is Phase 4. Include the method signature so Phase 4 fills it.)

- [ ] **Step 1: Write failing tests** — `body_map`: build over a 2-object synthetic snapshot, assert `get` returns the right `RoutineDecl` for a known `RoutineNodeId` and `None` for an absent one. `index`: build over a synthetic graph with a Table 18 "Customer", a TableExtension extending it, and a codeunit implementing interface "IFoo"; assert `object_by_number(from, Table, 18)` finds it within the closure and NOT outside; `table_extensions_of(customer)` returns the extension; `implementers_of("ifoo")` returns the codeunit.
- [ ] **Step 2: Run tests to verify they fail.**
- [ ] **Step 3: Implement `body_map.rs` + `index.rs`** — reuse `DependencyGraph::closure` for CallerClosure scoping (prefer-self then closure, like `resolve_object`); AnalyzedSnapshot iterates all apps. Build indexes from `graph.objects`/`graph.routines` (already NodeId-sorted) so lookups are deterministic.
- [ ] **Step 4: Run tests to verify they pass.**
- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/program/resolve/body_map.rs src/program/resolve/index.rs src/program/resolve/mod.rs
cargo clippy --release --all-features -- -D warnings 2>&1 | tail -3
git add src/program/resolve/body_map.rs src/program/resolve/index.rs src/program/resolve/mod.rs
git commit -m "feat(resolve): BodyMap + WorldMode-scoped ResolveIndex (Phase 1 Task 3)"
```

---

### Task 4: L3 PCallSite projection + Phase-1 site-parity gate

**Files:**
- Modify: `src/program/resolve/differential.rs`, `src/program/resolve/stub.rs` (switch the stub to structured `extract_sites` + tag site category), `tests/program_resolve_harness.rs`, `CHANGELOG.md`
- Test: `tests/program_resolve_harness.rs` (env-gated CDO)

**Interfaces:**
- Consumes: `extract_sites`/`CalleeShape` (Task 2), `project_l3` infra (Phase 0), L3 `L3Workspace.routines[*].call_sites` (PCallSite) read-only.
- Produces:
  - `pub fn project_l3_sites(workspace_root: &Path) -> Vec<CanonicalEdge>` — projects every L3 `PCallSite` (NOT CallEdge) to a `CanonicalEdge` with EMPTY targets (site-only), keyed identically to the fresh side (caller key + span + callee_fp of `cs.callee_text`). This is the SITE-level oracle.
  - Extend `DiffReport` with `extra_recordop: usize`, `extra_commit: usize`, `extra_unexplained: usize` (categorize fresh-only sites by `CalleeShape`).
  - `run_harness` (or a new `run_site_harness`) compares fresh `Bare|Member|ObjectRun` sites vs `project_l3_sites`, and categorizes fresh `RecordOp`/`Commit` sites as the justified extras.

- [ ] **Step 1: Write the failing CDO gate test**

```rust
#[test]
fn phase1_site_extraction_reconciles_with_l3() {
    let Some(ws) = std::env::var_os("CDO_WS").map(std::path::PathBuf::from).filter(|p| p.exists()) else { return; };
    let report = run_harness(&ws);
    // Fresh now classifies sites. Against L3 PCallSites, the CALL-category sites
    // (Bare/Member/ObjectRun) must reconcile: nothing missing, nothing unaligned,
    // and the ONLY unexplained extras are 0 (every extra is a record-op or commit).
    assert_eq!(report.missing_site, 0, "{report:?}");
    assert_eq!(report.unaligned, 0, "{report:?}");
    assert_eq!(report.extra_unexplained, 0, "every fresh-only call-category site must reconcile with an L3 PCallSite: {report:?}");
    // The record-op + commit extras account for the Phase-0 gap.
    assert!(report.extra_recordop > 0, "record-op sites should be a large justified-extra bucket: {report:?}");
    assert_eq!(report, run_harness(&ws), "deterministic");
}
```

> If `extra_unexplained > 0` after a faithful implementation, DO NOT relax the assertion — print the unexplained sites' `CalleeShape` + span and investigate: it means fresh's classification diverges from L3's (e.g. an implicit-Rec bare record-op not detected, or a member-vs-objectrun misclassification). Fix the classification, or — if a residual is provably a legitimate L3 modelling quirk — record it in a categorized, fixture-backed waiver (spec §6.4) and subtract it explicitly. A non-zero unexplained extra is the signal extraction isn't yet faithful.

- [ ] **Step 2: Run to verify it fails** — `cargo test -p al-call-hierarchy --test program_resolve_harness phase1_site` (compile/lookup error first).
- [ ] **Step 3: Implement** `project_l3_sites`, the `DiffReport` category fields, switch the stub/`run_harness` to structured extraction with category-aware bucketing. The site-level comparison uses the existing `match_sites` matcher.
- [ ] **Step 4: Run with CDO_WS set** — `CDO_WS="U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud" cargo test -p al-call-hierarchy --test program_resolve_harness phase1_site -- --nocapture 2>&1 | tail -20`. Confirm the body ran; print the breakdown; all asserts pass. If `extra_unexplained > 0`, investigate per Step 1's note before adjusting anything.
- [ ] **Step 5: Full gate + commit**

```bash
rustfmt src/program/resolve/differential.rs src/program/resolve/stub.rs tests/program_resolve_harness.rs
cargo clippy --release --all-features -- -D warnings 2>&1 | tail -3
cargo test --workspace 2>&1 | grep -E "test result:|FAILED" | tail -20
# CHANGELOG entry under [Unreleased]/Changed
git add src/program/resolve/differential.rs src/program/resolve/stub.rs tests/program_resolve_harness.rs CHANGELOG.md
git commit -m "feat(resolve): L3 PCallSite projection + Phase-1 site-parity gate (Phase 1 Task 4)"
```

---

## Roadmap — Phase 2 (next plan)

Core resolution + clean-room global-builtin catalog: resolve `Bare`/`ObjectRun` sites topology-scoped via `ResolveIndex` (self → extension chain → global-builtin catalog), emit real `Source`/`Abi`/`Catalog`/`Unknown` routes with §5.5 witnesses; implicit-trigger edges for the `Validate`/`Insert`/`Modify`/`Delete` record-ops (matching L3's `ImplicitTrigger` edges by anchor). Gate: REGRESSION/UNVERIFIED_EXTRA/EVIDENCE_OVERCLAIM == 0 on the in-scope subset; in-scope real-unknown ≤ L3. Then Phase 3 (receiver lattice + member builtins), Phase 4 (Polymorphic/Multicast fan-out), 1B.3 (ABI cross-check + retire L3).

## Self-Review

- **Spec coverage:** §5.1 structured extraction (Bare/Member/ObjectRun + record-op/commit classification) → Task 2; §4.1 ResolveIndex + WorldMode + BodyMap → Task 3; the harness reconciliation (categorized extra, site-level oracle) → Task 4; the two Phase-0-review prereqs → Task 1. Synthetic ImplicitTrigger/EventFlow EDGE emission is deferred to Phase 2/4 (Task 2 only CLASSIFIES record-op sites; it does not yet emit trigger edges) — correct staging.
- **Placeholder scan:** the "confirm the variant set / mirror `record_op.rs`/`record_types.rs`/`ir_walk.rs`" steps are bounded verification of code this plan must not guess at, each naming the exact file + action. No `TODO`/`add appropriate X`.
- **Type consistency:** `CalleeShape`/`RawSiteV2`/`record_op_names`/`routine_rvars`/`extract_sites` (Task 2) consumed by Task 4; `BodyMap`/`ResolveIndex`/`WorldMode` (Task 3) are Phase-2 substrate; `DiffReport` extra-category fields (Task 4) extend the Phase-0 struct; `make_canonical_key`/`object_kind_str`/`match_sites`/`project_l3` reused from Phase 0.
- **Known follow-ups (Phase 2+):** implicit-Rec bare record-op detection if Task 2 approximated it; event-subscriber index population; the `extra_recordop` sites become real ImplicitTrigger edges in Phase 2.
