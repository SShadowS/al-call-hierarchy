# Plan 1B.1 — Canonical Node Graph + App-Qualified Topology Index

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** From a parsed `AppSetSnapshot`, build a whole-program node graph where every AL object and routine has a canonical, **app-qualified** `NodeId` (so the same type+name in two different apps is two distinct nodes — never a flat-global collision), plus an **app-scoped** object index that resolves a name *within a given app's dependency closure* rather than the current flat, LAST-wins global table.

**Architecture:** A new `src/program/` module (the whole-program semantic graph). It consumes `snapshot::parse_snapshot(...) -> Vec<ParsedUnit>` (deep-parsed IR per app) and produces a `ProgramGraph` of `ObjectNode`/`RoutineNode`s keyed by app-qualified `NodeId`, with a `ProgramIndex` for app-scoped lookup. It does **not** touch the existing L3 resolver (`src/engine/l3/`) — that refactor/decision is Plan 1B.2 (edges). This plan only builds the node substrate + identity + scoping.

**Tech Stack:** Rust, the owned `al-syntax` IR (`al_syntax::ir::{AlFile, ObjectDecl, RoutineDecl, ObjectKind, RoutineKind}`), the merged `src/snapshot/` module, `string-interner` (already a dep, used by `graph.rs`) for symbol interning.

## Context — why this exists (grounded findings)

- The current `SymbolTable` (`src/engine/l3/symbol_table.rs`) keys objects `"${type_lc}/${name_lc}"` in **one global map across all apps**; on a name collision the LAST app assembled wins (`object_by_type_name` at `symbol_table.rs:231`). This is the soundness gap: a call cannot be soundly attributed to "the object from App A" vs "App B" by name alone.
- The owned IR has **no namespace field** on `ObjectDecl` — identity at this layer is `(app, kind, id|name)`.
- `parse_snapshot` already gives per-app parsed IR: `ParsedUnit{ app: AppId, files: Vec<ParsedFile{ virtual_path, file: AlFile, provenance }> }`.
- Plan 1B.2 (2-axis `Edge` + call resolution) and Plan 1B.3 (ABI cross-check + deep re-baseline) build on this node substrate. **Open design decision for 1B.2 (do NOT resolve here):** whether call resolution reuses the existing L3 pipeline (feed snapshot → L3 per app, make it app-scoped) or builds fresh over `ProgramGraph`. That decision is deferred to 1B.2's brainstorm.

## Global Constraints

- Rust edition 2024; toolchain pinned `rust-toolchain.toml` = 1.96.0. (verbatim)
- Format per-file with `rustfmt <file>`, never `cargo fmt`. Stage only intended paths; never `git add -A`.
- CI gates on `cargo clippy --release -- -D warnings`, `cargo fmt --check`, `cargo test --workspace` — leave all three green every task.
- No `unwrap()`/`expect()` on fallible paths reachable from a real workspace.
- Determinism: every collection that feeds output ordering is sorted by a stable key (`NodeId`), per the charter's determinism requirement. (The final-review follow-up "AppSetSnapshot.apps order is filesystem-dependent" is addressed here by sorting nodes by `NodeId`.)
- Dependency `AppId`s now carry their real unique GUID (from the `.app` NavxManifest `App@Id`; the `SnapshotBuilder` guid fix). Node identity still keys on the full `AppId` tuple (guid+name+publisher+version) as defensive identity — this is unique by guid in practice and still distinct by (name, publisher, version) for any residual empty-guid case.
- Real-IR facts (use verbatim): `AlFile{ objects: Vec<ObjectDecl>, ir: Ir, .. }`; `ObjectDecl{ kind: ObjectKind, id: Option<i64>, name: String, routines: Vec<RoutineDecl>, extends_target: Option<String>, implements: Vec<String>, .. }`; `RoutineDecl{ kind: RoutineKind, name: String, access_modifier: Option<String>, params: Vec<Param>, return_type: Option<String>, body: Option<BlockId>, .. }`; `ObjectKind{ Codeunit, Table, TableExtension, Page, .. , Interface, .. }`; `RoutineKind{ Procedure, Trigger }`.

---

### Task 1: `AppRef` interning + `NodeId` types

**Files:**
- Create: `src/program/mod.rs`
- Create: `src/program/node.rs`
- Modify: `src/lib.rs` (add `pub mod program;`)
- Modify: `CHANGELOG.md`
- Test: in-file `#[cfg(test)]` in `src/program/node.rs`

**Interfaces:**
- Produces:
  - `AppRef(u32)` — an interned handle for an `AppId` (Copy, Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd).
  - `ObjectNodeId { app: AppRef, kind: ObjectKind, key: ObjKey }` (Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)
  - `enum ObjKey { Id(i64), Name(String) }` (Ord etc.) — `Id` when the object has a number, else `Name` (extension objects / id-less).
  - `RoutineNodeId { object: ObjectNodeId, name_lc: String }` (Ord etc.) — `name_lc` is the lowercased routine name (AL is case-insensitive).
  - `AppRegistry { … }` with `fn intern(&mut self, app: &AppId) -> AppRef` and `fn resolve(&self, r: AppRef) -> &AppId`.

- [ ] **Step 1: Confirm module-root file** — Run `grep -n "pub mod snapshot" src/lib.rs`. Add `pub mod program;` to the same file (it is `src/lib.rs`).

- [ ] **Step 2: Write the failing test**
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use al_syntax::ir::ObjectKind;

    fn app(name: &str, ver: &str) -> crate::snapshot::AppId {
        crate::snapshot::AppId { guid: String::new(), name: name.into(),
            publisher: "P".into(), version: ver.into() }
    }

    #[test]
    fn app_ref_interns_by_full_identity_even_with_empty_guid() {
        let mut reg = AppRegistry::default();
        let a = reg.intern(&app("Core", "29.0.0.0"));
        let a2 = reg.intern(&app("Core", "29.0.0.0"));
        let b = reg.intern(&app("Core", "28.0.0.0")); // different version
        assert_eq!(a, a2);
        assert_ne!(a, b);
        assert_eq!(reg.resolve(a).name, "Core");
    }

    #[test]
    fn object_node_id_distinguishes_same_name_across_apps() {
        let mut reg = AppRegistry::default();
        let a = reg.intern(&app("AppA", "1.0.0.0"));
        let b = reg.intern(&app("AppB", "1.0.0.0"));
        let na = ObjectNodeId { app: a, kind: ObjectKind::Codeunit, key: ObjKey::Name("Sales-Post".into()) };
        let nb = ObjectNodeId { app: b, kind: ObjectKind::Codeunit, key: ObjKey::Name("Sales-Post".into()) };
        assert_ne!(na, nb, "same type+name in different apps must be distinct nodes");
    }
}
```

- [ ] **Step 3: Run test to verify it fails** — `cargo test -p al-call-hierarchy program::node 2>&1 | tail -15`. Expected: FAIL `cannot find type AppRegistry`.

- [ ] **Step 4: Write minimal implementation**

`src/program/node.rs`:
```rust
//! Canonical, app-qualified identity for whole-program graph nodes.

use al_syntax::ir::ObjectKind;
use crate::snapshot::AppId;
use std::collections::HashMap;

/// Interned handle for an `AppId` (cheap to copy/compare/sort).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct AppRef(pub u32);

/// Interns `AppId`s by their FULL identity (guid+name+publisher+version) — guid
/// is empty for deps today, so we never key on guid alone.
#[derive(Default)]
pub struct AppRegistry {
    by_key: HashMap<(String, String, String, String), AppRef>,
    apps: Vec<AppId>,
}

impl AppRegistry {
    pub fn intern(&mut self, app: &AppId) -> AppRef {
        let key = (
            app.guid.clone(),
            app.name.clone(),
            app.publisher.clone(),
            app.version.clone(),
        );
        if let Some(&r) = self.by_key.get(&key) {
            return r;
        }
        let r = AppRef(u32::try_from(self.apps.len()).expect("app arena overflow"));
        self.apps.push(app.clone());
        self.by_key.insert(key, r);
        r
    }

    pub fn resolve(&self, r: AppRef) -> &AppId {
        &self.apps[r.0 as usize]
    }
}

/// Object key: prefer the numeric id; fall back to the (lowercased) name for
/// id-less objects (extension objects, or where the IR has no number).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub enum ObjKey {
    Id(i64),
    Name(String),
}

/// Canonical identity of an AL object within one app.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct ObjectNodeId {
    pub app: AppRef,
    pub kind: ObjectKind,
    pub key: ObjKey,
}

/// Canonical identity of a routine within one object. `name_lc` is lowercased
/// (AL identifiers are case-insensitive).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct RoutineNodeId {
    pub object: ObjectNodeId,
    pub name_lc: String,
}
```
> `ObjectKind` must derive `Hash, Ord, PartialOrd` for these derives to compile. Check: `grep -n "pub enum ObjectKind" -A1 crates/al-syntax/src/ir/decl.rs` and its `#[derive(...)]`. If `Hash`/`Ord` are missing, add them to `ObjectKind`'s derive in `decl.rs` (it is a plain C-like enum — these derives are free and safe) and regen is NOT needed (decl.rs is hand-written, not generated). Note this in the commit.

`src/program/mod.rs`:
```rust
//! Whole-program semantic graph built from a parsed `AppSetSnapshot`
//! (charter §3). Plan 1B.1 = nodes + app-qualified identity + topology index.

pub mod node;

pub use node::{AppRef, AppRegistry, ObjKey, ObjectNodeId, RoutineNodeId};
```
Add `pub mod program;` to `src/lib.rs`. Add a CHANGELOG `### Added` bullet:
```markdown
- **Whole-program node graph** (`src/program/`) — app-qualified canonical
  `NodeId`s + topology index over the snapshot (Plan 1B.1).
```

- [ ] **Step 5: Run test to verify it passes** — `cargo test -p al-call-hierarchy program::node 2>&1 | tail -10`. Expected: PASS (2).

- [ ] **Step 6: Format, lint, commit**
```bash
rustfmt src/program/mod.rs src/program/node.rs
cargo clippy -p al-call-hierarchy --lib 2>&1 | tail -5
git add src/program/mod.rs src/program/node.rs src/lib.rs CHANGELOG.md crates/al-syntax/src/ir/decl.rs
git commit -m "feat(program): app-qualified NodeId + AppRegistry"
```
(Include `decl.rs` in the commit only if you added derives to `ObjectKind`.)

---

### Task 2: Object + routine node extraction from one `AppUnit`

**Files:**
- Create: `src/program/node_extract.rs`
- Modify: `src/program/mod.rs`
- Test: in-file `#[cfg(test)]` (uses `al_syntax::parse` on inline AL — no env fixture)

**Interfaces:**
- Consumes: `AppRef` (Task 1); `al_syntax::ir::{AlFile, ObjectDecl, RoutineDecl, ObjectKind, RoutineKind}`; `crate::snapshot::TrustTier`.
- Produces:
  - `struct ObjectNode { id: ObjectNodeId, name: String, declared_id: Option<i64>, extends_target: Option<String>, implements: Vec<String>, tier: TrustTier }`
  - `struct RoutineNode { id: RoutineNodeId, name: String, is_trigger: bool, access: Access, tier: TrustTier }`
  - `enum Access { Public, Local, Internal, Protected }` (from `RoutineDecl.access_modifier`)
  - `fn extract_nodes(app: AppRef, file: &AlFile, tier: TrustTier, objects: &mut Vec<ObjectNode>, routines: &mut Vec<RoutineNode>)`

- [ ] **Step 1: Write the failing test**
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::node::{AppRef, ObjKey};
    use crate::snapshot::TrustTier;

    #[test]
    fn extracts_object_and_routines_with_access() {
        let src = r#"
codeunit 50100 "Sales Helper"
{
    procedure Post() begin end;
    local procedure Helper() begin end;
}
"#;
        let file = al_syntax::parse(src);
        let mut objs = Vec::new();
        let mut routs = Vec::new();
        extract_nodes(AppRef(0), &file, TrustTier::Workspace, &mut objs, &mut routs);
        assert_eq!(objs.len(), 1);
        assert_eq!(objs[0].id.key, ObjKey::Id(50100));
        assert_eq!(objs[0].name, "Sales Helper");
        assert_eq!(routs.len(), 2);
        let post = routs.iter().find(|r| r.id.name_lc == "post").unwrap();
        assert_eq!(post.access, Access::Public);
        let helper = routs.iter().find(|r| r.id.name_lc == "helper").unwrap();
        assert_eq!(helper.access, Access::Local);
        assert!(!post.is_trigger);
    }
}
```

- [ ] **Step 2: Run test to verify it fails** — `cargo test -p al-call-hierarchy program::node_extract 2>&1 | tail -15`. Expected: FAIL `cannot find function extract_nodes`.

- [ ] **Step 3: Write minimal implementation**

`src/program/node_extract.rs`:
```rust
//! Extract object + routine nodes from one parsed `AlFile`.

use al_syntax::ir::{AlFile, RoutineKind};

use crate::program::node::{AppRef, ObjKey, ObjectNodeId, RoutineNodeId};
use crate::snapshot::TrustTier;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Access {
    Public,
    Local,
    Internal,
    Protected,
}

impl Access {
    fn from_modifier(m: Option<&str>) -> Access {
        match m.map(str::to_ascii_lowercase).as_deref() {
            Some("local") => Access::Local,
            Some("internal") => Access::Internal,
            Some("protected") => Access::Protected,
            _ => Access::Public,
        }
    }
}

pub struct ObjectNode {
    pub id: ObjectNodeId,
    pub name: String,
    pub declared_id: Option<i64>,
    pub extends_target: Option<String>,
    pub implements: Vec<String>,
    pub tier: TrustTier,
}

pub struct RoutineNode {
    pub id: RoutineNodeId,
    pub name: String,
    pub is_trigger: bool,
    pub access: Access,
    pub tier: TrustTier,
}

pub fn extract_nodes(
    app: AppRef,
    file: &AlFile,
    tier: TrustTier,
    objects: &mut Vec<ObjectNode>,
    routines: &mut Vec<RoutineNode>,
) {
    for obj in &file.objects {
        let key = match obj.id {
            Some(n) => ObjKey::Id(n),
            None => ObjKey::Name(obj.name.to_ascii_lowercase()),
        };
        let obj_id = ObjectNodeId {
            app,
            kind: obj.kind,
            key,
        };
        objects.push(ObjectNode {
            id: obj_id.clone(),
            name: obj.name.clone(),
            declared_id: obj.id,
            extends_target: obj.extends_target.clone(),
            implements: obj.implements.clone(),
            tier,
        });
        for r in &obj.routines {
            routines.push(RoutineNode {
                id: RoutineNodeId {
                    object: obj_id.clone(),
                    name_lc: r.name.to_ascii_lowercase(),
                },
                name: r.name.clone(),
                is_trigger: matches!(r.kind, RoutineKind::Trigger),
                access: Access::from_modifier(r.access_modifier.as_deref()),
                tier,
            });
        }
    }
}
```
Add `pub mod node_extract;` + re-exports (`ObjectNode, RoutineNode, Access, extract_nodes`) to `src/program/mod.rs`.

- [ ] **Step 4: Run test to verify it passes** — `cargo test -p al-call-hierarchy program::node_extract 2>&1 | tail -10`. Expected: PASS.

- [ ] **Step 5: Format, lint, commit**
```bash
rustfmt src/program/node_extract.rs src/program/mod.rs
cargo clippy -p al-call-hierarchy --lib 2>&1 | tail -5
git add src/program/node_extract.rs src/program/mod.rs
git commit -m "feat(program): extract object+routine nodes from AlFile (with access)"
```

---

### Task 3: `DependencyGraph` — per-app dependency closure (topology)

**Files:**
- Create: `src/program/topology.rs`
- Modify: `src/program/mod.rs`
- Test: in-file `#[cfg(test)]`

**Interfaces:**
- Consumes: `AppRef`, `AppRegistry` (Task 1).
- Produces:
  - `struct DependencyGraph { direct: std::collections::HashMap<AppRef, Vec<AppRef>> }`
  - `fn add_dependency(&mut self, from: AppRef, on: AppRef)`
  - `fn closure(&self, from: AppRef) -> std::collections::BTreeSet<AppRef>` — `from` plus all transitively reachable deps (an app sees itself + its dependency closure; cycle-safe).

> **Topology source:** every `AppUnit` now carries `declared_deps: Vec<AppDependency>` (each with its real GUID) — the workspace's from its `app.json`, each dependency app's from its `.app` NavxManifest `<Dependencies>` (the manifest-enrichment fix). So Task 4 wires the REAL per-app topology: for each app, `add_dependency(app_ref, dep_ref)` for every `declared_deps` entry resolved to an interned `AppRef` (match by GUID when present, else name+version). No workspace→all-deps workaround.

- [ ] **Step 1: Write the failing test**
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::node::AppRef;

    #[test]
    fn closure_includes_self_and_transitive_deps_cycle_safe() {
        let mut g = DependencyGraph::default();
        let (a, b, c) = (AppRef(0), AppRef(1), AppRef(2));
        g.add_dependency(a, b);
        g.add_dependency(b, c);
        g.add_dependency(c, a); // cycle — must not loop forever
        let cl = g.closure(a);
        assert!(cl.contains(&a) && cl.contains(&b) && cl.contains(&c));
        let only_c = g.closure(c);
        assert!(only_c.contains(&c) && only_c.contains(&a) && only_c.contains(&b));
    }
}
```

- [ ] **Step 2: Run test to verify it fails** — `cargo test -p al-call-hierarchy program::topology 2>&1 | tail -15`. Expected: FAIL.

- [ ] **Step 3: Write minimal implementation**

`src/program/topology.rs`:
```rust
//! App dependency topology: an app may reference objects in its own dependency
//! closure (itself + transitively declared dependencies), never the whole world.

use crate::program::node::AppRef;
use std::collections::{BTreeSet, HashMap};

#[derive(Default)]
pub struct DependencyGraph {
    direct: HashMap<AppRef, Vec<AppRef>>,
}

impl DependencyGraph {
    pub fn add_dependency(&mut self, from: AppRef, on: AppRef) {
        let deps = self.direct.entry(from).or_default();
        if !deps.contains(&on) {
            deps.push(on);
        }
    }

    /// `from` + all transitively reachable dependencies. Cycle-safe.
    pub fn closure(&self, from: AppRef) -> BTreeSet<AppRef> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![from];
        while let Some(a) = stack.pop() {
            if seen.insert(a) {
                if let Some(deps) = self.direct.get(&a) {
                    stack.extend(deps.iter().copied());
                }
            }
        }
        seen
    }
}
```
Add `pub mod topology;` + re-export `DependencyGraph` to `src/program/mod.rs`.

- [ ] **Step 4: Run test to verify it passes** — `cargo test -p al-call-hierarchy program::topology 2>&1 | tail -10`. Expected: PASS.

- [ ] **Step 5: Format, lint, commit**
```bash
rustfmt src/program/topology.rs src/program/mod.rs
cargo clippy -p al-call-hierarchy --lib 2>&1 | tail -5
git add src/program/topology.rs src/program/mod.rs
git commit -m "feat(program): DependencyGraph with cycle-safe app closure"
```

---

### Task 4: `ProgramGraph` + app-scoped object index + `build_program_graph`

**Files:**
- Create: `src/program/graph.rs`
- Create: `src/program/build.rs`
- Modify: `src/program/mod.rs`
- Test: in-file `#[cfg(test)]` (inline AL for the index/scope test; one env-guarded CDO test)

**Interfaces:**
- Consumes: everything above; `crate::snapshot::{AppSetSnapshot, parse_snapshot, ParsedUnit, ParsedFile, AppUnit}`.
- Produces:
  - `struct ProgramGraph { apps: AppRegistry, topology: DependencyGraph, objects: Vec<ObjectNode>, routines: Vec<RoutineNode>, obj_index: ObjectIndex }`
  - `struct ObjectIndex { by_app_kind_name: HashMap<(AppRef, ObjectKind, String), ObjectNodeId> }` (name lowercased)
  - `impl ProgramGraph { fn resolve_object(&self, from: AppRef, kind: ObjectKind, name: &str) -> Option<&ObjectNode> }` — searches `from`'s dependency closure (topology-aware), preferring the nearest app; returns the **app-qualified** node. Never a flat-global match.
  - `fn build_program_graph(snap: &AppSetSnapshot) -> ProgramGraph` — interns every `snap.apps[*].id`, extracts nodes from every source-bearing unit (via `parse_snapshot`), wires REAL topology from each `AppUnit.declared_deps` (resolve each declared dep to an interned `AppRef` by GUID-else-name+version; `add_dependency(app_ref, dep_ref)`), builds the index, sorts `objects`/`routines` by `NodeId` for determinism.

- [ ] **Step 1: Write the failing test** (topology scoping is the key assertion)
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use al_syntax::ir::ObjectKind;
    // Build a tiny ProgramGraph by hand to test resolve_object scoping without a workspace.
    // (Helper constructs AppRegistry + two apps + nodes + topology.)

    #[test]
    fn resolve_object_is_topology_scoped_not_global() {
        // App A depends on B. Both define codeunit "Util". A call from A must
        // resolve to A's own Util (nearest), and B cannot see A's Util at all.
        let g = build_two_app_fixture(); // see helper below
        let a = g.app_ref_by_name("AppA");
        let b = g.app_ref_by_name("AppB");
        let from_a = g.resolve_object(a, ObjectKind::Codeunit, "Util").unwrap();
        assert_eq!(from_a.id.app, a, "A resolves its own Util");
        let from_b = g.resolve_object(b, ObjectKind::Codeunit, "Util").unwrap();
        assert_eq!(from_b.id.app, b, "B resolves its own Util, never A's");
        // B does NOT depend on A, so an A-only object is invisible from B:
        assert!(g.resolve_object(b, ObjectKind::Codeunit, "OnlyInA").is_none());
    }
}
```
(Write a small `build_two_app_fixture()` test helper that constructs the `ProgramGraph` fields directly — interning AppA/AppB, adding `A depends on B`, inserting `Util` in both + `OnlyInA` in A, building the index. This isolates `resolve_object`'s scoping logic from snapshot I/O.)

- [ ] **Step 2: Run test to verify it fails** — `cargo test -p al-call-hierarchy program::graph 2>&1 | tail -15`. Expected: FAIL.

- [ ] **Step 3: Write minimal implementation**

`src/program/graph.rs` — `ProgramGraph`, `ObjectIndex`, and `resolve_object`:
```rust
//! The whole-program node graph: app-qualified nodes + topology-scoped lookup.

use al_syntax::ir::ObjectKind;
use std::collections::HashMap;

use crate::program::node::{AppRef, AppRegistry, ObjectNodeId};
use crate::program::node_extract::{ObjectNode, RoutineNode};
use crate::program::topology::DependencyGraph;

#[derive(Default)]
pub struct ObjectIndex {
    by_app_kind_name: HashMap<(AppRef, ObjectKind, String), usize>, // -> objects[idx]
}

pub struct ProgramGraph {
    pub apps: AppRegistry,
    pub topology: DependencyGraph,
    pub objects: Vec<ObjectNode>,
    pub routines: Vec<RoutineNode>,
    pub obj_index: ObjectIndex,
}

impl ProgramGraph {
    /// Resolve `(kind, name)` as seen FROM `from`: search `from`'s dependency
    /// closure, preferring the object declared in `from` itself, else any app
    /// in the closure (deterministic by `NodeId` order). Topology-scoped — an
    /// app outside the closure is never matched.
    pub fn resolve_object(
        &self,
        from: AppRef,
        kind: ObjectKind,
        name: &str,
    ) -> Option<&ObjectNode> {
        let name_lc = name.to_ascii_lowercase();
        let closure = self.topology.closure(from);
        // Prefer `from` itself.
        if let Some(&idx) = self.obj_index.by_app_kind_name.get(&(from, kind, name_lc.clone())) {
            return Some(&self.objects[idx]);
        }
        // Else the lowest-ordered app in the closure that declares it.
        let mut best: Option<usize> = None;
        for app in &closure {
            if let Some(&idx) = self.obj_index.by_app_kind_name.get(&(*app, kind, name_lc.clone())) {
                best = Some(match best {
                    Some(b) if self.objects[b].id <= self.objects[idx].id => b,
                    _ => idx,
                });
            }
        }
        best.map(|i| &self.objects[i])
    }
}
```
`src/program/build.rs` — `build_program_graph` (interns apps from `snap.apps`, runs `parse_snapshot`, `extract_nodes` per file with the unit's `provenance.tier`, wires `topology.add_dependency(workspace_ref, dep_ref)` for every dep, builds `ObjectIndex` from the collected `objects`, sorts `objects` and `routines` by `.id`). Re-export `ProgramGraph, ObjectIndex, build_program_graph` from `mod.rs`.
> Build the `ObjectIndex` AFTER sorting `objects` (so indices are stable), or build it from references and store the sorted position. On a same-app duplicate `(app,kind,name)` (should be rare/invalid AL), keep the first by `NodeId` order and record it (a later task can surface duplicates).

- [ ] **Step 4: Run test to verify it passes** — `cargo test -p al-call-hierarchy program::graph 2>&1 | tail -10`. Expected: PASS.

- [ ] **Step 5: Format, lint, commit**
```bash
rustfmt src/program/graph.rs src/program/build.rs src/program/mod.rs
cargo clippy -p al-call-hierarchy --lib 2>&1 | tail -5
git add src/program/graph.rs src/program/build.rs src/program/mod.rs
git commit -m "feat(program): ProgramGraph + topology-scoped object index + builder"
```

---

### Task 5: CDO integration + robustness gate

**Files:**
- Create: `tests/program_graph.rs`
- Test: `tests/program_graph.rs`

**Interfaces:**
- Consumes: `al_call_hierarchy::snapshot::SnapshotBuilder`, `al_call_hierarchy::program::build_program_graph`.

- [ ] **Step 1: Write the failing test**
```rust
//! Plan 1B.1: building the whole-program node graph over the real CDO snapshot
//! is panic-free and yields a deep, app-qualified node set.
#[test]
fn cdo_program_graph_is_app_qualified_and_panic_free() {
    let Some(ws) = std::env::var_os("CDO_WS").map(std::path::PathBuf::from)
        .filter(|p| p.exists()) else { return; };
    let snap = al_call_hierarchy::snapshot::SnapshotBuilder {
        workspace_root: ws, local_providers: vec![],
    }.build().expect("snapshot");
    let g = al_call_hierarchy::program::build_program_graph(&snap);
    // Deep node set across workspace + source-bearing deps.
    assert!(g.objects.len() > 500, "objects: {}", g.objects.len());
    assert!(g.routines.len() > 2000, "routines: {}", g.routines.len());
    // App-qualified: nodes span more than one app.
    let mut apps: std::collections::BTreeSet<_> = g.objects.iter().map(|o| o.id.app).collect();
    assert!(apps.len() >= 2, "nodes should span multiple apps, got {}", apps.len());
    // Deterministic: objects sorted by NodeId.
    assert!(g.objects.windows(2).all(|w| w[0].id <= w[1].id), "objects must be sorted by NodeId");
}
```
> Confirm the integration-test crate path: other `tests/*.rs` use `al_call_hierarchy::...`. Use that.

- [ ] **Step 2: Run test to verify it fails** — `cargo test -p al-call-hierarchy --test program_graph 2>&1 | tail -12`. Expected: FAIL (graph fns not reachable or counts off).

- [ ] **Step 3: Make it pass** — this should pass on the Task-4 implementation; if `build_program_graph` isn't `pub` at `crate::program::`, fix the re-export. Adjust the count thresholds only if the real numbers are below them AND you confirm (by printing) the graph genuinely covers the source-bearing deps (do not lower a threshold to hide a real gap — investigate first).

- [ ] **Step 4: Run test to verify it passes** — `CDO_WS="U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud" cargo test -p al-call-hierarchy --test program_graph 2>&1 | tail -10`. Expected: PASS.

- [ ] **Step 5: Full gate + commit**
```bash
rustfmt tests/program_graph.rs
cargo clippy --release --all-features -- -D warnings 2>&1 | tail -5
cargo test --workspace 2>&1 | grep -E 'test result:|FAILED' | tail -20
git add tests/program_graph.rs
git commit -m "test(program): CDO whole-program node-graph robustness + app-qualification gate"
```

---

## Self-Review

**Spec coverage (Plan 1B.1 vs Spec 1 §3.4 NodeId + the topology half of §3.5):**
- §3.4 canonical snapshot-qualified, namespace-aware NodeId → Tasks 1–2 (app-qualified `ObjectNodeId`/`RoutineNodeId`; namespace is absent from the IR — documented, identity is `(app,kind,id|name)`). ✓
- Dependency-topology-aware lookup (the fix for the flat-global `SymbolTable`) → Tasks 3–4 (`DependencyGraph.closure` + `resolve_object` scoped to the closure). ✓
- Determinism (charter C8 + the final-review dep-order follow-up) → Task 4 sorts by `NodeId`; Task 5 asserts sortedness. ✓
- **Deferred (correctly) to later 1B plans:** the 2-axis `Edge` structure + call resolution (§3.5 edges, §3.6 AbiCrossCheck) — Plan 1B.2/1B.3. The edge-resolution architecture (reuse L3 vs build-new) is an open design decision flagged in Context, NOT silently chosen here.

**Placeholder scan:** Two intentional "confirm with grep" verification steps (module-root in Task 1; `ObjectKind` derives in Task 1 Step 4; crate path in Task 5) — each has the exact command + action. No `TODO`/`add appropriate X`.

**Type consistency:** `AppRef`, `AppRegistry`, `ObjKey{Id,Name}`, `ObjectNodeId{app,kind,key}`, `RoutineNodeId{object,name_lc}`, `ObjectNode{id,name,declared_id,extends_target,implements,tier}`, `RoutineNode{id,name,is_trigger,access,tier}`, `Access`, `DependencyGraph` (`add_dependency`/`closure`), `ProgramGraph{apps,topology,objects,routines,obj_index}`, `ObjectIndex`, `resolve_object`, `build_program_graph` — consistent across Tasks 1–5.

**Known follow-ups (later 1B):** transitive per-dep topology edges (parse each `.app` manifest's deps — today only workspace→deps is wired); same-app duplicate-object surfacing; namespace as an identity discriminant if BC namespace collisions appear; the `ObjectKind` `Hash/Ord` derive add (if needed) ripples nowhere but note it.

## Next plans (not this one)
- **Plan 1B.2 — 2-axis `Edge` + call resolution** over `ProgramGraph` (walk routine bodies for `ExprKind::Call`, resolve callee topology-aware, emit `(DispatchShape, Vec<Route>)`). *Requires a design decision first: reuse the existing L3 resolver (make it app-scoped) vs build fresh over `ProgramGraph`.*
- **Plan 1B.3 — ABI cross-check verifier + deep re-baseline** (verify resolved routes against `AppUnit.abi` SymbolReference; stratified real-`unknown` metric over the deep graph).
