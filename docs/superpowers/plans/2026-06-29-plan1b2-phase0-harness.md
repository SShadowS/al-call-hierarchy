# Plan 1B.2 Phase 0 — Edge Model + Dual-Run Harness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the multi-axis `Edge` data model and the span-based dual-run differential harness that diffs a (stub) fresh resolver against the existing L3 resolver on the CDO corpus — the foundation Phases 1–4 resolve into.

**Architecture:** New `src/program/resolve/` module over the 1B.1 `ProgramGraph`. This phase delivers the `Edge` types + obligation metric (`edge.rs`), a canonical projection both engines target, an L3→canonical adapter, a span-based site **matcher** (ordinals abolished) with an `UNALIGNED` bucket, the diff-bucket engine, a minimal stub `resolve_program` emitting one `Unknown` obligation per call expression, and an `aldump --program-call-graph-stats` that runs the harness end-to-end and measures the full gap. No real resolution yet — that is Phases 2–4.

**Tech Stack:** Rust (edition 2024, toolchain 1.96.0), the `al-call-hierarchy` crate, `al_syntax` owned IR, `rayon`, the existing `src/engine/l3` resolver kept as the oracle.

**Source of truth:** `docs/superpowers/specs/2026-06-29-plan1b2-fresh-resolver-design.md` (§3 edge model, §3.2 obligations, §6 oracle). Read it before starting.

## Global Constraints

- Rust edition 2024; toolchain pinned 1.96.0. Format per-file with `rustfmt <file>` — **never** `cargo fmt`.
- Stage only the files each task names — **never** `git add -A` / `git add .`.
- CI gates every commit: `cargo clippy --release --all-features -- -D warnings`, `cargo fmt --check`, `cargo test --workspace`. All three must pass.
- **The existing `src/engine/l3` resolver and its goldens are the ORACLE — do not modify them.** This phase only reads L3 output.
- Determinism: all emitted collections sorted by a stable key; two runs byte-identical. Reuse 1B.1's filesystem-independent `AppRef` ordering (`load_all_apps` sorts deps by `AppId`).
- CDO fixture is env-gated: tests that need it read `CDO_WS` (value `U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud`) and **return early when unset** — so they MUST be run with the env var set to genuinely assert; a skipped run is not a pass.
- Update `CHANGELOG.md` under `## [Unreleased]` → `### Added` once per task that adds a user-visible capability (the new module, the new aldump flag).
- New `src/program/resolve/*` modules consistently use `to_ascii_lowercase` for name folding (matches 1B.1 `src/program/`), and reference 1B.1 types from `crate::program::node` / `crate::program::node_extract`.

## Module / file structure (this phase)

| File | Responsibility |
|------|----------------|
| `src/program/resolve/mod.rs` | `pub mod` wiring; re-export `Edge`, `resolve_program`, the harness entry. |
| `src/program/resolve/edge.rs` | `Edge`, `EdgeKind`, `DispatchShape`, `SetCompleteness`, `OpenWorldReason`, `Route`, `RouteTarget`, `Evidence`, `Condition`, `Witness`, `BuiltinId`, `SiteId`, `SourcePos`, `CanonicalSpan`; obligation accounting + `real_unknown_rate` + `Histogram`. |
| `src/program/resolve/extract_min.rs` | Minimal IR call-expression walk → `RawSite` (span + callee text), used by the stub resolver. (Phase 1 replaces with structured extraction.) |
| `src/program/resolve/stub.rs` | `resolve_program` stub: one `Unknown`-obligation `Edge` per `RawSite`. |
| `src/program/resolve/differential.rs` | `CanonicalEdge` projection; L3→canonical adapter; the span-based site matcher + `UNALIGNED`; the diff-bucket engine + `DiffReport`. |
| `tests/program_resolve_harness.rs` | Env-gated CDO end-to-end harness gate + the matcher fixture matrix. |
| `src/bin/aldump.rs` (modify) | Add `--program-call-graph-stats` invoking the harness. |

---

### Task 1: Edge model core types

**Files:**
- Create: `src/program/resolve/mod.rs`
- Create: `src/program/resolve/edge.rs`
- Modify: `src/program/mod.rs` (add `pub mod resolve;`)
- Test: in `src/program/resolve/edge.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes (from 1B.1, `crate::program::node`): `AppRef` (Copy+Ord+Hash), `ObjectNodeId`, `RoutineNodeId` (both Clone+Eq+Hash+Ord).
- Produces: the full `edge.rs` type surface used by every later task — names exactly as below.

- [ ] **Step 1: Write the failing test**

In `src/program/resolve/edge.rs`, append:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::node::{AppRef, ObjKey, ObjectKind, ObjectNodeId, RoutineNodeId};

    fn rid(app: u32, name: &str) -> RoutineNodeId {
        RoutineNodeId {
            object: ObjectNodeId { app: AppRef(app), kind: ObjectKind::Codeunit, key: ObjKey::Id(1) },
            name_lc: name.to_string(),
        }
    }

    #[test]
    fn edge_constructs_and_is_orderable() {
        let e = Edge {
            from: rid(0, "post"),
            site: SiteId {
                caller: rid(0, "post"),
                span: CanonicalSpan { unit: "u".into(), start: SourcePos { line: 1, col: 1 }, end: SourcePos { line: 1, col: 9 } },
                callee_fingerprint: 42,
            },
            kind: EdgeKind::Call,
            shape: DispatchShape::Exact,
            completeness: SetCompleteness::Complete,
            routes: vec![Route {
                target: RouteTarget::Routine(rid(0, "helper")),
                evidence: Evidence::Source,
                condition: None,
                witness: Witness::SourceSpan { file: "f.al".into(), span: (10, 20) },
            }],
        };
        assert_eq!(e.routes.len(), 1);
        // Hashable + comparable (needed by the differential).
        let mut v = vec![e.clone(), e];
        v.sort();
        assert_eq!(v.len(), 2);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p al-call-hierarchy program::resolve::edge 2>&1 | tail -15`
Expected: FAIL — `edge` module / types not found (won't compile).

- [ ] **Step 3: Write the types**

Create `src/program/resolve/mod.rs`:

```rust
//! Plan 1B.2: fresh call/behaviour-edge resolver over `ProgramGraph`.
//! Phase 0 = edge model + dual-run differential harness (this module set).

pub mod differential;
pub mod edge;
pub mod extract_min;
pub mod stub;

pub use edge::{
    DispatchShape, Edge, EdgeKind, Evidence, ObligationOutcome, Route, RouteTarget,
    SetCompleteness, Witness, real_unknown_rate,
};
pub use stub::resolve_program;
```

Add `pub mod resolve;` to `src/program/mod.rs` (next to the existing `pub mod` lines).

Create `src/program/resolve/edge.rs`:

```rust
//! The multi-axis behaviour-edge model + the obligation-based real-unknown metric.
//! Spec §3 / §3.2.

use crate::program::node::{AppRef, RoutineNodeId};

/// Caller / target identity is a 1B.1 app-qualified routine node.
pub type NodeId = RoutineNodeId;

/// A platform builtin's catalog identity (clean-room catalog id; Phase 2+).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BuiltinId(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourcePos {
    pub line: u32,
    pub col: u32,
}

/// Line/col span in a named source unit — the coordinate BOTH engines align on
/// (L3 records line/col via `PAnchor`; the fresh side converts IR byte-origins).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CanonicalSpan {
    pub unit: String,
    pub start: SourcePos,
    pub end: SourcePos,
}

/// Stable SEMANTIC identity of an originating site (spec §6.1) — span-based, never positional.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SiteId {
    pub caller: NodeId,
    pub span: CanonicalSpan,
    pub callee_fingerprint: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EdgeKind {
    Call,
    Run,
    ImplicitTrigger,
    EventFlow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DispatchShape {
    Exact,
    Polymorphic,
    Multicast,
    DynamicOpen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OpenWorldReason {
    ReverseDependentImplementers,
    ReverseDependentSubscribers,
    ReverseDependentExtensions,
    RuntimeTypeUnbounded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SetCompleteness {
    /// Provably exhaustive (sealed / closed-world snapshot) — NOT merely "enumerated the snapshot".
    Complete,
    /// Open world may add routes; also the edge-level home of a DynamicOpen blocker
    /// (`RuntimeTypeUnbounded`) and of a legal empty fan-out.
    Partial { reason: OpenWorldReason },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Evidence {
    Source,
    Abi,
    Catalog,
    /// ABI body-unavailable boundary ONLY — never a visibility conclusion (spec §5.4).
    Opaque,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Condition {
    RunTriggerGuarded,
    ManualBinding,
    SkipOnMissingLicense,
    SkipOnMissingPermission,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RouteTarget {
    Routine(NodeId),
    Builtin(BuiltinId),
    /// Known public boundary whose body is unavailable — retains symbol identity.
    AbiSymbol { app: AppRef, symbol_key: String },
    /// Genuine failure only — pairs with `Evidence::Unknown`.
    Unresolved,
}

/// Independent-checkability handle for a route's evidence (spec §5.5).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Witness {
    SourceSpan { file: String, span: (u32, u32) },
    AbiSymbol { app: AppRef, symbol_key: String },
    CatalogEntry { id: BuiltinId, catalog_version: String },
    None,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Route {
    pub target: RouteTarget,
    pub evidence: Evidence,
    pub condition: Option<Condition>,
    pub witness: Witness,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Edge {
    pub from: NodeId,
    pub site: SiteId,
    pub kind: EdgeKind,
    pub shape: DispatchShape,
    pub completeness: SetCompleteness,
    pub routes: Vec<Route>,
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p al-call-hierarchy program::resolve::edge 2>&1 | tail -8`
Expected: PASS (`edge_constructs_and_is_orderable`).

- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/program/resolve/mod.rs src/program/resolve/edge.rs src/program/mod.rs
cargo clippy --release --all-features -- -D warnings 2>&1 | tail -3
git add src/program/resolve/mod.rs src/program/resolve/edge.rs src/program/mod.rs
git commit -m "feat(resolve): multi-axis Edge model (Plan 1B.2 Phase 0 Task 1)"
```

> Note: `mod.rs` references `differential`/`extract_min`/`stub` modules created in later tasks. To keep Task 1 compiling, create those three files as empty stubs (`//! placeholder` + nothing else) in this task and stage them too — later tasks fill them. Add them to the `git add` line.

---

### Task 2: Obligation accounting + real-unknown metric

**Files:**
- Modify: `src/program/resolve/edge.rs` (append `ObligationOutcome`, `classify_obligation`, `real_unknown_rate`, `Histogram`)
- Test: `src/program/resolve/edge.rs` tests

**Interfaces:**
- Consumes: `Edge`, `DispatchShape`, `EdgeKind`, `SetCompleteness`, `Evidence`, `RouteTarget`, `Witness` (Task 1).
- Produces: `ObligationOutcome` (Resolved/HonestDynamic/HonestEmpty/Unknown), `fn classify_obligation(&Edge) -> ObligationOutcome`, `fn real_unknown_rate(&[Edge]) -> f64`, `struct Histogram` + `fn Histogram::of_edges(&[Edge]) -> Histogram`.

- [ ] **Step 1: Write the failing test**

Append to the `tests` mod in `edge.rs`:

```rust
fn edge_with(kind: EdgeKind, shape: DispatchShape, comp: SetCompleteness, routes: Vec<Route>) -> Edge {
    Edge {
        from: rid(0, "c"),
        site: SiteId { caller: rid(0, "c"),
            span: CanonicalSpan { unit: "u".into(), start: SourcePos { line: 1, col: 1 }, end: SourcePos { line: 1, col: 2 } },
            callee_fingerprint: 1 },
        kind, shape, completeness: comp, routes,
    }
}
fn src_route() -> Route {
    Route { target: RouteTarget::Routine(rid(0, "t")), evidence: Evidence::Source, condition: None,
        witness: Witness::SourceSpan { file: "f".into(), span: (0, 1) } }
}

#[test]
fn obligation_outcomes_are_correct() {
    // Resolved: >=1 non-Unknown route.
    assert_eq!(classify_obligation(&edge_with(EdgeKind::Call, DispatchShape::Exact, SetCompleteness::Complete, vec![src_route()])), ObligationOutcome::Resolved);
    // HonestDynamic: DynamicOpen.
    assert_eq!(classify_obligation(&edge_with(EdgeKind::Run, DispatchShape::DynamicOpen, SetCompleteness::Partial { reason: OpenWorldReason::RuntimeTypeUnbounded }, vec![])), ObligationOutcome::HonestDynamic);
    // HonestEmpty: fan-out, zero routes, Partial.
    assert_eq!(classify_obligation(&edge_with(EdgeKind::EventFlow, DispatchShape::Multicast, SetCompleteness::Partial { reason: OpenWorldReason::ReverseDependentSubscribers }, vec![])), ObligationOutcome::HonestEmpty);
    // Unknown: Exact Call with no target.
    assert_eq!(classify_obligation(&edge_with(EdgeKind::Call, DispatchShape::Exact, SetCompleteness::Complete, vec![])), ObligationOutcome::Unknown);
    // Metric: 1 Unknown out of 4 obligations.
    let edges = vec![
        edge_with(EdgeKind::Call, DispatchShape::Exact, SetCompleteness::Complete, vec![src_route()]),
        edge_with(EdgeKind::Run, DispatchShape::DynamicOpen, SetCompleteness::Partial { reason: OpenWorldReason::RuntimeTypeUnbounded }, vec![]),
        edge_with(EdgeKind::EventFlow, DispatchShape::Multicast, SetCompleteness::Partial { reason: OpenWorldReason::ReverseDependentSubscribers }, vec![]),
        edge_with(EdgeKind::Call, DispatchShape::Exact, SetCompleteness::Complete, vec![]),
    ];
    assert!((real_unknown_rate(&edges) - 0.25).abs() < 1e-9);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p al-call-hierarchy program::resolve::edge::tests::obligation_outcomes_are_correct 2>&1 | tail -10`
Expected: FAIL — `classify_obligation` / `ObligationOutcome` / `real_unknown_rate` not found.

- [ ] **Step 3: Implement the accounting**

Append to `edge.rs` (before the `tests` mod):

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObligationOutcome {
    Resolved,
    HonestDynamic,
    HonestEmpty,
    Unknown,
}

/// Classify one edge's resolution obligation (spec §3.2).
pub fn classify_obligation(e: &Edge) -> ObligationOutcome {
    let has_real_route = e
        .routes
        .iter()
        .any(|r| r.evidence != Evidence::Unknown && r.target != RouteTarget::Unresolved);
    if has_real_route {
        return ObligationOutcome::Resolved;
    }
    if e.shape == DispatchShape::DynamicOpen {
        return ObligationOutcome::HonestDynamic;
    }
    let is_fanout = matches!(e.shape, DispatchShape::Polymorphic | DispatchShape::Multicast);
    let is_open = matches!(e.completeness, SetCompleteness::Partial { .. });
    if e.routes.is_empty() && is_fanout && is_open {
        return ObligationOutcome::HonestEmpty;
    }
    ObligationOutcome::Unknown
}

/// real-unknown = Unknown obligations / all obligations (spec §3.2).
pub fn real_unknown_rate(edges: &[Edge]) -> f64 {
    if edges.is_empty() {
        return 0.0;
    }
    let unknown = edges
        .iter()
        .filter(|e| classify_obligation(e) == ObligationOutcome::Unknown)
        .count();
    unknown as f64 / edges.len() as f64
}

/// Stratified counts for `--program-call-graph-stats`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Histogram {
    pub total: usize,
    pub resolved: usize,
    pub honest_dynamic: usize,
    pub honest_empty: usize,
    pub unknown: usize,
}

impl Histogram {
    pub fn of_edges(edges: &[Edge]) -> Histogram {
        let mut h = Histogram::default();
        for e in edges {
            h.total += 1;
            match classify_obligation(e) {
                ObligationOutcome::Resolved => h.resolved += 1,
                ObligationOutcome::HonestDynamic => h.honest_dynamic += 1,
                ObligationOutcome::HonestEmpty => h.honest_empty += 1,
                ObligationOutcome::Unknown => h.unknown += 1,
            }
        }
        h
    }
    pub fn real_unknown_rate(&self) -> f64 {
        if self.total == 0 { 0.0 } else { self.unknown as f64 / self.total as f64 }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p al-call-hierarchy program::resolve::edge 2>&1 | tail -8`
Expected: PASS (both edge tests).

- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/program/resolve/edge.rs
cargo clippy --release --all-features -- -D warnings 2>&1 | tail -3
git add src/program/resolve/edge.rs
git commit -m "feat(resolve): obligation accounting + real-unknown metric (Phase 0 Task 2)"
```

---

### Task 3: Minimal call-expression extraction (`extract_min`)

**Files:**
- Modify: `src/program/resolve/extract_min.rs`
- Test: `src/program/resolve/extract_min.rs` tests

**Interfaces:**
- Consumes: `al_syntax` IR — `al_syntax::parse(&str) -> AlFile`; `AlFile.objects: Vec<ObjectDecl>`; each object's routines (`RoutineDecl { name, body: Option<BlockId>, origin, .. }`); the IR arena to walk blocks/stmts/exprs for `ExprKind::Call`/`StmtKind::Call`; `Origin` byte ranges. Confirm the exact IR accessors by reading `crates/al-syntax/src/ir/` (`expr.rs`, `stmt.rs`, `decl.rs`) and how `src/engine/l2/ir_walk.rs` walks them — mirror that traversal, do not call it.
- Produces: `struct RawSite { caller_routine: String /*name_lc*/, callee_text: String, span: CanonicalSpan }`; `fn extract_raw_sites(file: &al_syntax::ir::AlFile, unit: &str) -> Vec<RawSite>`. `CanonicalSpan` from `edge.rs`. Byte-origin → line/col conversion via the source text (`fn byte_to_pos(src: &str, byte: usize) -> SourcePos`).

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_one_site_per_call_expression() {
        let src = r#"
codeunit 50100 "C"
{
    procedure Run()
    begin
        Foo();
        Bar(1, 2);
    end;
    procedure Foo() begin end;
    procedure Bar(a: Integer; b: Integer) begin end;
}
"#;
        let file = al_syntax::parse(src);
        let sites = extract_raw_sites(&file, "C.al");
        // Two call expressions in Run(): Foo() and Bar(1,2).
        let in_run: Vec<_> = sites.iter().filter(|s| s.caller_routine == "run").collect();
        assert_eq!(in_run.len(), 2, "sites: {sites:?}");
        assert!(in_run.iter().any(|s| s.callee_text.to_ascii_lowercase().contains("foo")));
        // Spans are non-degenerate and ordered by source position.
        assert!(in_run[0].span.start <= in_run[1].span.start);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p al-call-hierarchy program::resolve::extract_min 2>&1 | tail -12`
Expected: FAIL — `extract_raw_sites` not found.

- [ ] **Step 3: Implement the minimal walk**

Implement `extract_raw_sites` in `extract_min.rs`: for each object, for each routine with a body, walk the IR block/stmt/expr arena collecting every `ExprKind::Call { function, .. }` (and `StmtKind::Call`), recording the callee's source text and converting the call expr's `Origin` byte-range to a `CanonicalSpan` via `byte_to_pos` over the routine's source. Keep `caller_routine` = routine `name` lowercased. **Read `crates/al-syntax/src/ir/expr.rs`, `stmt.rs`, and `src/engine/l2/ir_walk.rs` first** to use the correct arena-access API; the exact `AlFile`/arena accessor names are the only unknowns — confirm them there, do not invent. Sort the returned `Vec<RawSite>` by `(caller_routine, span.start)` for determinism.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p al-call-hierarchy program::resolve::extract_min 2>&1 | tail -8`
Expected: PASS.

- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/program/resolve/extract_min.rs
cargo clippy --release --all-features -- -D warnings 2>&1 | tail -3
git add src/program/resolve/extract_min.rs
git commit -m "feat(resolve): minimal call-expression extraction (Phase 0 Task 3)"
```

---

### Task 4: Stub resolver + canonical projection (fresh side)

**Files:**
- Modify: `src/program/resolve/stub.rs`, `src/program/resolve/differential.rs`
- Test: `src/program/resolve/differential.rs` tests

**Interfaces:**
- Consumes: `RawSite`/`extract_raw_sites` (Task 3); `Edge` + types (Task 1); 1B.1 `build_program_graph`, `parse_snapshot`, `ParsedUnit { app, files }`, `ParsedFile { virtual_path, file }`.
- Produces:
  - `fn resolve_program(snap: &AppSetSnapshot) -> Vec<Edge>` (stub): parse units, extract raw sites, emit one `Edge` per site with `kind: Call`, `shape: Exact`, `completeness: Complete`, `routes: vec![Route { target: Unresolved, evidence: Unknown, condition: None, witness: Witness::None }]`. The caller `NodeId` is resolved from the routine's object via the ProgramGraph; if the caller object can't be keyed, skip the site (counted later as a known limitation).
  - `struct CanonicalTarget { kind: u8, app: Option<String>, object_lc: String, routine_lc: Option<String> }` (Ord+Hash); `struct CanonicalEdge { from: CanonicalKey, site: CanonicalSiteKey, kind: EdgeKind, targets: BTreeSet<CanonicalTarget> }`; `struct CanonicalKey { app_guid: String, object_kind: String, object_lc: String, routine_lc: String }`; `struct CanonicalSiteKey { caller: CanonicalKey, span: CanonicalSpan, callee_fp: u64 }`.
  - `fn project_fresh(edges: &[Edge], apps: &AppRegistry) -> Vec<CanonicalEdge>` (maps `AppRef`→guid via the registry).

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_fresh_round_trips_a_synthetic_edge() {
        // Build a tiny ProgramGraph-free CanonicalEdge directly from a synthetic Edge.
        // (Full CDO projection is exercised by the env-gated harness test, Task 7.)
        let edges = crate::program::resolve::stub::synthetic_unknown_edge_for_test();
        let apps = crate::program::node::AppRegistry::default();
        let canon = project_fresh(&edges, &apps);
        assert_eq!(canon.len(), 1);
        assert!(canon[0].targets.is_empty(), "stub Unknown edge has no concrete target");
    }
}
```

Add a small `#[cfg(test)] pub fn synthetic_unknown_edge_for_test() -> Vec<Edge>` to `stub.rs` returning one stub `Edge` (Unknown route) using a fabricated `RoutineNodeId`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p al-call-hierarchy program::resolve::differential::tests::project_fresh_round_trips_a_synthetic_edge 2>&1 | tail -12`
Expected: FAIL — `project_fresh` / stub helpers not found.

- [ ] **Step 3: Implement stub + projection**

Implement `resolve_program` (stub) in `stub.rs` and `CanonicalEdge`/`project_fresh` in `differential.rs` per the interface above. An `Unresolved`/`Unknown` route projects to an **empty** `targets` set (so the diff sees "fresh resolved nothing here"). Map `AppRef`→guid through `AppRegistry` (1B.1 — confirm the accessor that yields an app's `AppId.guid` from an `AppRef`; add a small read-only helper on `AppRegistry` if none exists, in a separate commit-safe edit).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p al-call-hierarchy program::resolve::differential 2>&1 | tail -8`
Expected: PASS.

- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/program/resolve/stub.rs src/program/resolve/differential.rs
cargo clippy --release --all-features -- -D warnings 2>&1 | tail -3
git add src/program/resolve/stub.rs src/program/resolve/differential.rs
git commit -m "feat(resolve): stub resolver + fresh canonical projection (Phase 0 Task 4)"
```

---

### Task 5: L3 → canonical adapter (oracle side)

**Files:**
- Modify: `src/program/resolve/differential.rs`
- Test: `src/program/resolve/differential.rs` tests (env-gated on a fixture or CDO)

**Interfaces:**
- Consumes (READ-ONLY, the oracle): `crate::engine::l3::l3_workspace::assemble_and_resolve_workspace_default(&Path) -> Option<L3Resolved>`; `L3Resolved.workspace: L3Workspace`; `crate::engine::l3::symbol_table::SymbolTable::build(&objects, &tables, &routines)`; `crate::engine::l3::call_resolver::{resolve_calls, ResolvedCalls, CallEdge}`; `CallEdge { from: String, to: Option<String>, callsite_id: String, operation_id: String, .. }`; `L3Routine.call_sites: Vec<PCallSite>` where `PCallSite { id, operation_id, callee_text, source_anchor: PAnchor { source_unit_id, start_line, start_column, end_line, end_column, .. }, .. }`. **Confirm the exact module paths + the `L3Workspace` accessors for objects/tables/routines by reading `src/engine/l3/l3_workspace.rs` and `src/bin/aldump.rs:816` (the existing `--l3-call-graph-stats` path) — mirror how aldump builds the SymbolTable and calls `resolve_calls`.**
- Produces: `fn project_l3(workspace_root: &Path) -> Vec<CanonicalEdge>` — runs L3, builds a `callsite_id -> &PCallSite` map (for the span), maps each `CallEdge` to a `CanonicalEdge` (parse the internal `from`/`to` id strings `${appGuid}/${objectType}/${objectNumber}/...` into `CanonicalKey`/`CanonicalTarget`; the span comes from the matched `PCallSite.source_anchor`). `L3 to == None` ⇒ empty `targets`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn project_l3_yields_spanned_canonical_edges_on_cdo() {
    let Some(ws) = std::env::var_os("CDO_WS").map(std::path::PathBuf::from).filter(|p| p.exists()) else { return; };
    let edges = project_l3(&ws);
    assert!(edges.len() > 1000, "L3 should project many edges, got {}", edges.len());
    // Every projected site carries a real span (non-zero end).
    assert!(edges.iter().all(|e| e.site.span.end.line >= e.site.span.start.line));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p al-call-hierarchy program::resolve::differential::tests::project_l3_yields_spanned_canonical_edges_on_cdo 2>&1 | tail -12`
Expected: FAIL — `project_l3` not found. (Body would be skipped without `CDO_WS`; failure here is the compile/lookup error.)

- [ ] **Step 3: Implement the adapter**

Implement `project_l3`. Parse the L3 id-string format (confirm it against `src/engine/l3/call_resolver.rs` projection code / `call_graph_projection.rs`). Build the `callsite_id -> PCallSite` index from `workspace.routines[*].call_sites`. Convert `PAnchor` (line/col, 1-based or 0-based — confirm) into `CanonicalSpan`.

- [ ] **Step 4: Run test to verify it passes**

Run: `CDO_WS="U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud" cargo test -p al-call-hierarchy program::resolve::differential::tests::project_l3_yields_spanned_canonical_edges_on_cdo -- --nocapture 2>&1 | tail -8`
Expected: PASS, with a printed edge count > 1000. Confirm the body RAN (not skipped).

- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/program/resolve/differential.rs
cargo clippy --release --all-features -- -D warnings 2>&1 | tail -3
git add src/program/resolve/differential.rs
git commit -m "feat(resolve): L3->canonical oracle adapter (Phase 0 Task 5)"
```

---

### Task 6: Span-based site matcher + `UNALIGNED` (the critical-risk task)

**Files:**
- Modify: `src/program/resolve/differential.rs`
- Test: `tests/program_resolve_harness.rs` (the matcher fixture matrix — no env needed)

**Interfaces:**
- Consumes: `CanonicalEdge`, `CanonicalSiteKey`, `CanonicalSpan` (Task 4).
- Produces: `enum SiteMatch { Paired(usize, usize), FreshOnly(usize), L3Only(usize), Unaligned(Vec<usize>, Vec<usize>) }`; `fn match_sites(fresh: &[CanonicalEdge], l3: &[CanonicalEdge]) -> Vec<SiteMatch>`. Algorithm (spec §6.1): partition both by `(caller, kind)`; within a partition, match by (1) exact `span+callee_fp`, (2) span-overlap + `callee_fp`, (3) span-overlap only as a single-candidate fallback; remaining unmatched on either side that cannot be paired 1:1 → `Unaligned`. **A site that fails to match becomes one `Unaligned`, never shifting its neighbours' matches.**

- [ ] **Step 1: Write the failing test (the cascade-resistance proof)**

```rust
//! Phase 0: the dual-run site matcher must NOT cascade — one extra/missing site
//! produces exactly one UNALIGNED, not a routine-wide divergence storm.
use al_call_hierarchy::program::resolve::differential::*;

#[test]
fn one_missing_site_does_not_cascade() {
    // Build 5 fresh sites at increasing spans; L3 has the same 5 minus the 2nd.
    let mk = |start: u32, fp: u64| canonical_call_edge_for_test("cu:c:run", start, fp);
    let fresh = vec![mk(10, 1), mk(20, 2), mk(30, 3), mk(40, 4), mk(50, 5)];
    let l3    = vec![mk(10, 1),            mk(30, 3), mk(40, 4), mk(50, 5)];
    let matches = match_sites(&fresh, &l3);
    let paired = matches.iter().filter(|m| matches!(m, SiteMatch::Paired(_, _))).count();
    let fresh_only = matches.iter().filter(|m| matches!(m, SiteMatch::FreshOnly(_))).count();
    // 4 clean pairs; the 2nd fresh site is the single FreshOnly; NO cascade on 3/4/5.
    assert_eq!(paired, 4, "matches: {matches:?}");
    assert_eq!(fresh_only, 1);
}
```

Add a `#[cfg(test)] pub fn canonical_call_edge_for_test(caller: &str, span_start: u32, fp: u64) -> CanonicalEdge` helper to `differential.rs`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p al-call-hierarchy --test program_resolve_harness one_missing_site_does_not_cascade 2>&1 | tail -12`
Expected: FAIL — `match_sites` not found.

- [ ] **Step 3: Implement the matcher**

Implement `match_sites` with the partition + tiered-match + `Unaligned` algorithm. Crucially, match greedily within `(caller, kind)` by sorted span and require span+fp agreement; an unmatched fresh site is `FreshOnly`, an unmatched L3 site is `L3Only`, and only genuinely ambiguous leftovers (multiple plausible pairings) become `Unaligned`. The test above asserts the no-cascade property.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p al-call-hierarchy --test program_resolve_harness one_missing_site_does_not_cascade 2>&1 | tail -8`
Expected: PASS. Add 2–3 more matcher fixtures (duplicate spans same line; chained member calls producing nested spans; a fresh-only synthetic-trigger site that has no L3 peer → `FreshOnly`, not cascade) in the same file and confirm all pass.

- [ ] **Step 5: Format, lint, commit**

```bash
rustfmt src/program/resolve/differential.rs tests/program_resolve_harness.rs
cargo clippy --release --all-features -- -D warnings 2>&1 | tail -3
cargo test -p al-call-hierarchy --test program_resolve_harness 2>&1 | grep -E "test result:"
git add src/program/resolve/differential.rs tests/program_resolve_harness.rs
git commit -m "feat(resolve): span-based site matcher with UNALIGNED, no cascade (Phase 0 Task 6)"
```

---

### Task 7: Diff buckets + `aldump --program-call-graph-stats` + CDO gap gate

**Files:**
- Modify: `src/program/resolve/differential.rs` (the `DiffReport` bucket engine + a `run_harness(workspace_root) -> DiffReport`)
- Modify: `src/bin/aldump.rs` (add the `--program-call-graph-stats` flag)
- Modify: `CHANGELOG.md`
- Test: `tests/program_resolve_harness.rs` (env-gated CDO end-to-end)

**Interfaces:**
- Consumes: `match_sites` (Task 6), `project_fresh`/`resolve_program` (Task 4), `project_l3` (Task 5), `Histogram`/`real_unknown_rate` (Task 2), 1B.1 `SnapshotBuilder`/`build_program_graph`.
- Produces: `struct DiffReport { fresh: Histogram, l3_edges: usize, matched: usize, regression: usize, missing_site: usize, extra_site: usize, unaligned: usize, /* Phase-0: every paired site is REGRESSION since fresh is all-Unknown */ }`; `fn run_harness(workspace_root: &Path) -> DiffReport`. The full bucket set (VERIFIED_WIN / UNVERIFIED_EXTRA / EVIDENCE_OVERCLAIM / DIVERGENCE) is wired but trivially zero until real resolution (Phases 2–4).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn harness_runs_end_to_end_on_cdo_and_measures_the_gap() {
    let Some(ws) = std::env::var_os("CDO_WS").map(std::path::PathBuf::from).filter(|p| p.exists()) else { return; };
    let report = run_harness(&ws);
    // L3 oracle has many edges; fresh stub extracted many sites.
    assert!(report.l3_edges > 1000, "{report:?}");
    assert!(report.fresh.total > 1000, "{report:?}");
    // Matcher health: alignment failures are a small fraction (the stub still extracts real call
    // expressions, so most sites should PAIR even though fresh resolved nothing).
    let pair_or_single = report.matched + report.missing_site + report.extra_site;
    assert!(report.unaligned * 20 < pair_or_single.max(1),
        "UNALIGNED must be <5% of sites, got {} of {}", report.unaligned, pair_or_single);
    // Phase-0 baseline: fresh is all-Unknown, so every matched site is a REGRESSION.
    assert_eq!(report.regression, report.matched, "stub fresh resolves nothing → all matches regress");
    // Determinism: two runs identical.
    assert_eq!(report, run_harness(&ws));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p al-call-hierarchy --test program_resolve_harness harness_runs_end_to_end_on_cdo_and_measures_the_gap 2>&1 | tail -12`
Expected: FAIL — `run_harness` / `DiffReport` not found.

- [ ] **Step 3: Implement the diff engine + aldump flag**

Implement `run_harness`: build the snapshot + ProgramGraph, run `resolve_program` (fresh) → `project_fresh`, run `project_l3` (oracle), `match_sites`, then bucket each match (Phase 0: a `Paired` site where fresh `targets` is empty and L3 `targets` non-empty = `REGRESSION`; `FreshOnly` = `EXTRA-SITE`; `L3Only` = `MISSING-SITE`; `Unaligned` counted). Compute the fresh `Histogram`. Add `--program-call-graph-stats <workspace>` to `aldump.rs` printing the `DiffReport` + `fresh.real_unknown_rate()` (mirror the existing `--l3-call-graph-stats` arg plumbing).

- [ ] **Step 4: Run test to verify it passes**

Run: `CDO_WS="U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud" cargo test -p al-call-hierarchy --test program_resolve_harness harness_runs_end_to_end_on_cdo_and_measures_the_gap -- --nocapture 2>&1 | tail -12`
Expected: PASS; printed report shows L3 edges, fresh sites, the REGRESSION gap (= the work Phases 2–4 close), and UNALIGNED well under 5%. Confirm the body RAN.

- [ ] **Step 5: Format, lint, full gate, commit**

```bash
rustfmt src/program/resolve/differential.rs src/bin/aldump.rs
cargo clippy --release --all-features -- -D warnings 2>&1 | tail -3
cargo test --workspace 2>&1 | grep -E "test result:|FAILED" | tail -20
# add a CHANGELOG entry under [Unreleased]/Added for the resolve module + aldump flag
git add src/program/resolve/differential.rs src/bin/aldump.rs CHANGELOG.md
git commit -m "feat(resolve): dual-run diff harness + --program-call-graph-stats (Phase 0 Task 7)"
```

---

## Roadmap — subsequent plans (Phases 1–4)

Each gets its OWN plan document (written when reached, informed by the harness's real output from Phase 0). Named here with deliverable + gate so the sequence is legible. **Do not implement these from this document.**

- **Phase 1 — Structured extraction + `ResolveIndex` + `BodyMap`** (`docs/.../plan1b2-phase1-extraction.md`).
  Replace `extract_min` with structured site extraction (Bare/Member/ObjectRun + synthetic `ImplicitTrigger`/`EventFlow` sites + arity + receiver `ExprId`); build the lookup indexes (routine-overload, object-by-number, table, table-extension-by-base, interface/enum-implementer, event-subscriber) with explicit `WorldMode` (CallerClosure vs AnalyzedSnapshot — spec §4.1); build `BodyMap` (`NodeId -> &RoutineDecl`). **Gate (Call/Run domain):** MISSING-SITE == 0, EXTRA-SITE justified, UNALIGNED == 0 vs L3 on CDO.
- **Phase 2 — Core resolution + clean-room global-builtin catalog (generator-driven)** (spec §5.3, §5.6).
  Bare/self/extension-chain/object-run resolution + global builtins; the catalog generator against a pinned compiler surface + diff-vs-existing. Emit real `Source`/`Abi`/`Catalog`/`Opaque`/`Unknown` routes with §5.5 witnesses. **Gate (Bare/Run in-scope):** REGRESSION / UNVERIFIED_EXTRA / EVIDENCE_OVERCLAIM == 0; in-scope real-unknown ≤ L3.
- **Phase 3 — Receiver-type lattice + member-builtin catalog** (spec §5.2).
  Phase A inference (locals/params/globals/Rec/xRec/CurrPage/with-stack/trigger-context/return-types/Variant/RecordRef/FieldRef/subtypes) + Phase B dispatch; member builtins. **Gate (Member in-scope):** same predicates on the member subset.
- **Phase 4 — Polymorphic + Multicast edges, open-world completeness, whole-corpus gate** (spec §3.1, §5.3).
  Interface/enum (`Polymorphic`), event/extension-trigger/implicit-trigger (`Multicast`) with `SetCompleteness::Partial` open-world tails + `Condition`s. **Gate:** whole-corpus, all filters removed — REGRESSION / UNALIGNED / UNVERIFIED_EXTRA / EVIDENCE_OVERCLAIM == 0, all DIVERGENCEs adjudicated (machine-checkable, spec §6.4), `fresh.real_unknown_rate <= l3.real_unknown_rate` stratified.

(Then Plan 1B.3: full SymbolReference ABI cross-check, deep re-baseline, retire the L3 oracle.)

## Self-Review

- **Spec coverage (Phase 0 scope):** §3 edge model → Task 1; §3.2 obligations/metric → Task 2; §5.1 minimal extraction → Task 3; stub resolver + fresh projection (§4, §6.1) → Task 4; L3 adapter (§6.2) → Task 5; span-based matcher + UNALIGNED (§6.1) → Task 6; diff buckets + `--program-call-graph-stats` + CDO gap gate (§6.3, §6.5) → Task 7. Phases 1–4 spec sections are explicitly deferred to their own plans (roadmap), not omitted.
- **Placeholder scan:** the three "confirm the exact IR/L3 accessor by reading <file>" steps (Tasks 3/4/5) are bounded verification of the ONLY unknowns (arena/accessor method names in code this plan must not guess at), each with the file to read and the action — not open-ended TODOs. No "add error handling"/"etc." placeholders.
- **Type consistency:** `Edge`/`SiteId`/`CanonicalSpan`/`SourcePos`/`Route`/`RouteTarget`/`Evidence`/`Witness`/`ObligationOutcome`/`Histogram` (Tasks 1–2) are used verbatim by Tasks 3–7; `CanonicalEdge`/`CanonicalKey`/`CanonicalSiteKey`/`CanonicalTarget` (Task 4) used by Tasks 5–7; `match_sites`/`SiteMatch` (Task 6) used by Task 7. Names are consistent across tasks.
