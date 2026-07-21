# L4 Summary-Fixpoint Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the string-keyed, provenance-carrying Jacobi fixpoint that computes L4 db-effect
summaries (one 797-member SCC = 729s, ~40 GB peak on the 8020 Base App) with an interned bitvector
solver: PD product-graph reachability + closed-form per-effective-SCC union + on-demand `via`, so
`compute_summaries` drops to seconds with byte-identical output.

**Architecture:** A new `db_effect_solver` computes each routine's db-effect set as a closed-form
union over *effective* SCCs (Tarjan SCCs re-decomposed after removing fixed leaves / missing
routines), with the only per-member flow — `ParameterDependent` substitution — solved as a
semi-naive reachability worklist over an interned effect universe; `via` and the side-facts
(`uncertainties`, `has_unresolved_calls`) are reconstructed by separate monotone passes. The new
solver is introduced behind the existing `compute_summaries_with_leaves` seam and gated by a
complete-`RoutineSummary` differential against the old Jacobi solver before cutover.

**Tech Stack:** Rust, `rayon` (existing), the L4 modules under `src/engine/l4/`. No new dependencies.

## Global Constraints

- **Phase 1 is STRICT PARITY.** New solver output MUST equal the old over the complete internal
  `RoutineSummary` (`db_effects` incl. `record_variable_id`, `uncertainties`, `has_unresolved_calls`,
  `parameter_roles`), per routine. `RoutineSummary` derives `PartialEq, Eq` — compare directly. Any
  Phase-1 divergence is a bug, never a rebaseline.
- **Phase 2 is a SEPARATELY APPROVED soundness change** with explicit rebaseline — not in this
  performance-preserving arc. Do not touch `parameter_roles` semantics in Phase 1.
- **The per-pass `RawSccTrace` is not a contract** — update its oracle to compare final semantics, or
  keep the old Jacobi solver behind `#[cfg(test)]` for trace-compat only.
- **Engine-never-throws.** No new production panics; convergence backstops are debug-only diagnostics.
- **Determinism.** Interned-ID materialization MUST reproduce the existing `(effect_key,
  operation_id)` sort (`summary_runner.rs:507-510`).
- **Batch path first; Salsa deferred.** The 8020 hot path is `compute_summaries_with_leaves`
  (`detector_context.rs:510`, `771`; `summary.rs:795`; `capability_cone.rs:2100,2587`). The Salsa
  incremental path (`incremental/queries.rs:728`, LSP-only) stays on the old solver until Task 13.
- **rustfmt per file (never `cargo fmt`). Stage only intended paths (never `git add -A`). Never
  push/merge to master without explicit request.** (CLAUDE.md.)
- **Package name is `al-call-hierarchy` (hyphen)** for `cargo test -p`.

---

## Existing interfaces (verbatim — do not redefine, consume these)

```rust
// src/engine/l4/summary.rs
pub enum TempState { Known(bool), ParameterDependent(u32), Unknown }           // :29
pub struct DbEffect {                                                          // :55
    pub effect_key: String, pub operation_id: String, pub op: String,
    pub table_id: String, pub record_variable_id: Option<String>,
    pub temp_state: TempState, pub via: String,
}
pub struct Uncertainty {                                                       // :69
    pub kind: String, pub callsite_id: Option<String>, pub operation_id: Option<String>,
    pub routine_id: Option<String>, pub interface_name: Option<String>,
}
pub struct RoutineSummary {                                                    // :165
    pub routine_id: String, pub db_effects: Vec<DbEffect>, pub in_recursive_cycle: bool,
    pub has_unresolved_calls: bool, pub uncertainties: Vec<Uncertainty>,
    pub parameter_roles: Vec<RecordRoleSummary>,
}
pub fn uncertainty_key(u: &Uncertainty) -> String;                            // :98

// src/engine/l4/effect_lattice.rs
pub enum TempStateKind { Known(bool), ParameterDependent(u32), Unknown }       // :76
impl TempStateKind { pub fn key_fragment(&self) -> String; }                   // :90
pub fn effect_key_of(op:&str, table_id:&str, operation_id:&str, ts:&TempStateKind) -> String; // :122
pub fn merge_via<'a>(a:&'a str, b:&'a str) -> &'a str;                         // :147
pub fn via_for_edge_kind(kind:&str) -> &'static str;                          // :169
impl TempState { pub fn to_kind(&self) -> TempStateKind; }                     // summary.rs:45

// src/engine/l4/combined_graph.rs
pub struct CombinedGraph { pub nodes: Vec<String>,                             // :110
    pub edges_by_from: HashMap<String, Vec<CombinedEdge>>, pub edges_from_order: Vec<String>,
    pub uncertainty_edges: Vec<UncertaintyEdge>, pub typed_edges: Vec<TypedEdge> }
pub struct CombinedEdge { pub from:String, pub to:String, pub kind:String,     // :39
    pub callsite_id:Option<String>, pub operation_id:Option<String>, pub event_id:Option<String>,
    pub subscriber_app_id:Option<String>, pub resolution:String }
pub struct UncertaintyEdge { pub from:String, pub uncertainty:Uncertainty }    // :54

// src/engine/l4/scc.rs
pub struct Scc { pub members: Vec<String>, pub recursive: bool }               // :32
pub struct SccResult { pub sccs: Vec<Scc>, pub scc_id_by_routine: HashMap<String,usize> } // :39
pub struct SccInputGraph<'a> { pub nodes:&'a [String], pub edges_by_from:&'a HashMap<String,Vec<String>> } // :23
pub fn tarjan_scc(graph: &SccInputGraph) -> SccResult;                        // :59

// src/engine/l4/summary_runner.rs
pub type FieldIndex = HashMap<(String,String), String>;                        // :322
pub fn base_intraprocedural_summary(r:&L3Routine, by_id:&HashMap<String,&L3Routine>, f:&FieldIndex) -> RoutineSummary; // :104
pub fn compute_summaries_with_leaves(routines:&[L3Routine], graph:&CombinedGraph, scc:&SccResult,
    upgraded_bindings:&HashMap<String,Vec<UpgradedBinding>>, fields:&FieldIndex, collect_trace:bool,
    leaf_summaries:&HashMap<String,RoutineSummary>)
  -> (HashMap<String,RoutineSummary>, Vec<RawSccTrace>, Vec<SummarizeDiagnostic>);  // :846
fn compose_routine(routine:&L3Routine, snapshot:&HashMap<String,RoutineSummary>,
    final_map:&HashMap<String,RoutineSummary>, base_summaries:&HashMap<String,RoutineSummary>,
    upgraded_bindings:&HashMap<String,Vec<UpgradedBinding>>, graph:&CombinedGraph,
    body_avail_by_id:&HashMap<String,bool>, uncertainty_edges_by_from:&HashMap<String,Vec<usize>>)
  -> RoutineSummary;   // :351  — the OLD transfer function; Phase 1 keeps it for roles + as differential oracle
```

**Key fact (verified):** no `src/engine/l5/**` detector reads `RoutineSummary.db_effects`. Internal
`db_effects` is consumed only by (a) `compose_routine` (`:375,404`) and (b) the projection
`project_db_effect` (`summary.rs:545`) → `PRoutineFullSummary.db_effects` (`capability_cone.rs:2650`).
So the detector-facing contract is the PROJECTED `PDbEffect`; the internal representation is free to
change as long as `RoutineSummary` stays `Eq`-identical.

---

## File structure

- **Create `src/engine/l4/effect_universe.rs`** — `EffectUniverse`: intern structured effect identity
  `(op, table_id, operation_id, TempStateKind)` ↔ `EffectId(u32)`; frozen; `id → materialized DbEffect
  fields`; `sorted_permutation()` reproducing `(effect_key, operation_id)` order.
- **Create `src/engine/l4/db_effect_solver.rs`** — Steps 0–D + side-facts: `effective_sccs`,
  `solve_pd_reachability`, `closed_form_union`, `reconstruct_via`, `solve_side_facts`, and the
  `solve_scc_db_effects` assembly returning per-member `(db_effects, uncertainties,
  has_unresolved_calls)`.
- **Modify `src/engine/l4/summary_runner.rs`** — add `compute_summaries_v2_with_leaves` (same
  signature) that uses the new solver for db_effects/uncertainties/has_unresolved_calls and the
  existing `compose_routine` for `parameter_roles`; keep the old fn as the differential oracle.
- **Create `tests/l4_summary_differential.rs`** — generated-SCC fixtures + complete-`RoutineSummary`
  comparison of v2 vs old; the TDD spine for every solver task.
- **Modify `tests/perf_bounds.rs`** — add a `compute_summaries` order-of-magnitude bound.

---

## Phase 1 — db_effects redesign (STRICT PARITY)

### Task 1: Differential harness + `compute_summaries_v2` seam

**Files:**
- Create: `tests/l4_summary_differential.rs`
- Modify: `src/engine/l4/summary_runner.rs` (add `compute_summaries_v2_with_leaves`, initially
  delegating to the old fn), `src/engine/l4/mod.rs` (ensure `pub mod` exposure if needed)
- Test: `tests/l4_summary_differential.rs`

**Interfaces:**
- Produces: `pub fn compute_summaries_v2_with_leaves(routines:&[L3Routine], graph:&CombinedGraph,
  scc:&SccResult, upgraded_bindings:&HashMap<String,Vec<UpgradedBinding>>, fields:&FieldIndex,
  leaf_summaries:&HashMap<String,RoutineSummary>) -> HashMap<String,RoutineSummary>` (no trace tuple —
  the v2 path drops the trajectory artifact; diagnostics handled in Task 10).
- A fixture builder `fn scc_fixture(name:&str) -> (Vec<L3Routine>, CombinedGraph, SccResult, FieldIndex, HashMap<String,Vec<UpgradedBinding>>)`.

- [ ] **Step 1: Write the failing differential test** with the smallest fixture (two routines, one
  calls the other, one `Known(true)` base effect each). Assert v2 == old over complete `RoutineSummary`:

```rust
// tests/l4_summary_differential.rs
use al_call_hierarchy::engine::l4::summary_runner::{
    compute_summaries_with_leaves, compute_summaries_v2_with_leaves,
};
use std::collections::HashMap;

/// Compare v2 against the old Jacobi solver over the COMPLETE RoutineSummary.
fn assert_parity(name: &str) {
    let (routines, graph, scc, fields, ub) = fixtures::build(name);
    let leaves = HashMap::new();
    let (old, _t, _d) =
        compute_summaries_with_leaves(&routines, &graph, &scc, &ub, &fields, false, &leaves);
    let new = compute_summaries_v2_with_leaves(&routines, &graph, &scc, &ub, &fields, &leaves);
    assert_eq!(old.len(), new.len(), "[{name}] routine count");
    for (id, old_s) in &old {
        let new_s = new.get(id).unwrap_or_else(|| panic!("[{name}] missing {id}"));
        assert_eq!(old_s, new_s, "[{name}] summary mismatch for {id}");
    }
}

#[test]
fn parity_linear_two_routine_known_effect() {
    assert_parity("linear_known");
}
```

  Build `fixtures::build` in the same file (a `mod fixtures`) constructing real `L3Routine`,
  `CombinedGraph`, `SccResult`, `FieldIndex`, and `upgraded_bindings` for the named case. Reuse
  `src/engine/l5/test_support.rs` constructors where they exist (it already builds `RecordRoleSummary`
  / `DbEffect` skeletons — see `test_support.rs:269,307`). The `linear_known` fixture: routine `A`
  (id `"a"`) with one `Insert` op on table `t1` (`Known(true)` temp), routine `B` (id `"b"`) that
  calls `A` via a `direct` `CombinedEdge`; `SccResult` = two singleton non-recursive SCCs in
  reverse-topo order `[a, b]`.

- [ ] **Step 2: Run it — expect FAIL to compile** (`compute_summaries_v2_with_leaves` undefined).

```bash
cargo test -p al-call-hierarchy --test l4_summary_differential parity_linear -- --nocapture
```
Expected: compile error `cannot find function compute_summaries_v2_with_leaves`.

- [ ] **Step 3: Add `compute_summaries_v2_with_leaves` delegating to the old solver** so the harness
  goes green and the seam exists:

```rust
// src/engine/l4/summary_runner.rs
/// v2 db-effect solver seam. Task 1 delegates to the old Jacobi solver so the
/// differential harness is green from day one; Tasks 2-8 replace the db-effect /
/// uncertainty / has_unresolved_calls computation with the new solver, keeping
/// parameter_roles from the old `compose_routine`.
pub fn compute_summaries_v2_with_leaves(
    routines: &[L3Routine],
    graph: &CombinedGraph,
    scc: &SccResult,
    upgraded_bindings: &HashMap<String, Vec<UpgradedBinding>>,
    fields: &FieldIndex,
    leaf_summaries: &HashMap<String, RoutineSummary>,
) -> HashMap<String, RoutineSummary> {
    let (map, _trace, _diag) = compute_summaries_with_leaves(
        routines, graph, scc, upgraded_bindings, fields, false, leaf_summaries,
    );
    map
}
```

- [ ] **Step 4: Run the harness — expect PASS.**

```bash
cargo test -p al-call-hierarchy --test l4_summary_differential -- --nocapture
```
Expected: `parity_linear_two_routine_known_effect ... ok`.

- [ ] **Step 5: Add the full fixture matrix** (each a `#[test] fn parity_*` calling `assert_parity`),
  still green because v2 delegates. These fixtures are the acceptance set every later task must keep
  green:
  - `linear_known` (done), `recursive_self_loop` (A calls A, one PD effect),
  - `recursive_pair_pd` (A↔B, a `ParameterDependent(0)` effect that re-symbolizes),
  - `pd_to_known` (PD substituted to `Known(true)` via a `temporary`-keyword binding),
  - `pd_to_unknown` (PD with no captured source temp state → `Unknown`),
  - `multi_callsite_same_callee` (A calls B twice with different bindings),
  - `via_collision` (same effect_key reached via `direct` and `event-dispatch` edges → max rank),
  - `external_successor_pd` (recursive SCC whose member calls an already-settled successor carrying a
    PD effect),
  - `fixed_leaf_in_scc` (a 3-member Tarjan cycle where one member is a fixed leaf, passed in
    `leaf_summaries`),
  - `missing_routine_in_scc` (a member id present in `scc.members` but absent from `routines`).

- [ ] **Step 6: Commit.**

```bash
git add tests/l4_summary_differential.rs src/engine/l4/summary_runner.rs
git commit -m "test(l4): differential harness + compute_summaries_v2 seam (delegating)"
```

---

### Task 2: `EffectUniverse` interner

**Files:**
- Create: `src/engine/l4/effect_universe.rs`
- Modify: `src/engine/l4/mod.rs` (add `pub mod effect_universe;`)
- Test: inline `#[cfg(test)] mod tests` in `effect_universe.rs`

**Interfaces:**
- Produces:
```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct EffectId(pub u32);

/// Structured effect identity — the interned key. Excludes `via` (provenance) and
/// `record_variable_id` (non-key payload), matching `effect_key_of` (effect_lattice.rs:122).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct EffectIdentity {
    pub op: String, pub table_id: String, pub operation_id: String,
    pub temp: TempStateKind,
}

pub struct EffectUniverse { /* interner: identity -> EffectId, plus reverse Vec */ }
impl EffectUniverse {
    pub fn new() -> Self;
    /// Intern (creating on first sight). Lazily grows as PD substitution invents variants.
    pub fn intern(&mut self, id: &EffectIdentity) -> EffectId;
    pub fn get(&self, id: &EffectIdentity) -> Option<EffectId>;
    pub fn identity(&self, id: EffectId) -> &EffectIdentity;
    pub fn len(&self) -> usize;
    /// Deterministic materialization order: EffectIds sorted by (effect_key, operation_id),
    /// reproducing summary_runner.rs:507-510. Returns EffectIds in that order.
    pub fn sorted_order(&self) -> Vec<EffectId>;
    /// The full effect_key string for an id (lazy — only for materialization/projection).
    pub fn effect_key(&self, id: EffectId) -> String; // effect_key_of(op,table,opid,temp)
}
```

- [ ] **Step 1: Write failing test** — intern determinism + sorted order reproduces `(effect_key,
  operation_id)`:

```rust
#[test]
fn intern_is_deterministic_and_sorted_order_matches_effect_key() {
    use crate::engine::l4::effect_lattice::TempStateKind;
    let mut u = EffectUniverse::new();
    let a = EffectIdentity { op:"Insert".into(), table_id:"t2".into(),
        operation_id:"op9".into(), temp: TempStateKind::Known(true) };
    let b = EffectIdentity { op:"Insert".into(), table_id:"t1".into(),
        operation_id:"op1".into(), temp: TempStateKind::Known(true) };
    let ia = u.intern(&a);
    let ib = u.intern(&b);
    assert_eq!(u.intern(&a), ia, "re-intern stable");
    assert_ne!(ia, ib);
    // sorted_order must be by effect_key then operation_id, NOT insertion order.
    let order = u.sorted_order();
    let keys: Vec<String> = order.iter().map(|&e| u.effect_key(e)).collect();
    let mut expected = keys.clone();
    expected.sort();
    assert_eq!(keys, expected, "sorted_order yields effect_key ascending");
}
```

- [ ] **Step 2: Run — expect FAIL** (`cannot find EffectUniverse`).

```bash
cargo test -p al-call-hierarchy --lib effect_universe::tests -- --nocapture
```

- [ ] **Step 3: Implement `EffectUniverse`** with a `HashMap<EffectIdentity, EffectId>` + reverse
  `Vec<EffectIdentity>`; `sorted_order` sorts all ids by `(effect_key(id), operation_id)`; `effect_key`
  delegates to `effect_key_of(op, table_id, operation_id, &temp)`.

- [ ] **Step 4: Run — expect PASS.**

- [ ] **Step 5: Commit.**

```bash
git add src/engine/l4/effect_universe.rs src/engine/l4/mod.rs
git commit -m "feat(l4): EffectUniverse interner (structured identity <-> EffectId, sorted materialization)"
```

---

### Task 3: Effective-SCC re-decomposition (Step 0)

**Files:**
- Create: `src/engine/l4/db_effect_solver.rs` (with this fn + a `#[cfg(test)] mod tests`)
- Modify: `src/engine/l4/mod.rs` (`pub mod db_effect_solver;`)

**Interfaces:**
- Consumes: `Scc` (scc.rs:32), `tarjan_scc`/`SccInputGraph` (scc.rs), `CombinedGraph` (edges_by_from).
- Produces:
```rust
/// An effective SCC: a strongly-connected component of the graph induced by REMOVING
/// fixed leaves and missing routines from `scc_entry.members`. One Tarjan Scc can yield
/// several effective SCCs (a removed leaf can split a cycle into DAG parts). Returned in
/// reverse-topological order (callees first), matching tarjan_scc's contract.
pub fn effective_sccs(
    scc_entry: &Scc,
    graph: &CombinedGraph,
    is_recomputed: &dyn Fn(&str) -> bool, // true iff member is NEITHER a fixed leaf NOR missing
) -> Vec<Scc>;
```

- [ ] **Step 1: Write failing tests** for the two split cases:

```rust
#[test]
fn fixed_leaf_splits_cycle_into_dag_parts() {
    // Tarjan SCC {a,b,c} with edges a->b->c->a. Mark `b` as NOT recomputed (fixed leaf).
    // Induced graph over {a,c}: a-> (b removed) , c->a  => edges: c->a only. No cycle.
    // Expect TWO singleton effective SCCs [c],[a]? reverse-topo: callee-first.
    let graph = build_cycle_graph(&["a","b","c"], &[("a","b"),("b","c"),("c","a")]);
    let scc = Scc { members: vec!["a".into(),"b".into(),"c".into()], recursive: true };
    let eff = effective_sccs(&scc, &graph, &|id| id != "b");
    // b excluded; a and c are now acyclic (c->a). Two non-recursive singletons.
    let members: Vec<Vec<String>> = eff.iter().map(|s| s.members.clone()).collect();
    assert_eq!(eff.len(), 2, "leaf removal splits the cycle");
    assert!(eff.iter().all(|s| !s.recursive));
    // reverse-topo: c before a (c calls a)
    assert_eq!(members, vec![vec!["c".to_string()], vec!["a".to_string()]]);
}

#[test]
fn missing_routine_excluded_same_as_leaf() {
    let graph = build_cycle_graph(&["a","b"], &[("a","b"),("b","a")]);
    let scc = Scc { members: vec!["a".into(),"b".into()], recursive: true };
    let eff = effective_sccs(&scc, &graph, &|id| id != "b"); // b missing
    assert_eq!(eff.len(), 1);
    assert_eq!(eff[0].members, vec!["a".to_string()]);
    assert!(!eff[0].recursive);
}
```

  (Provide `build_cycle_graph(nodes, edges) -> CombinedGraph` in the test module: build
  `edges_by_from` as `CombinedEdge`s with `kind:"direct"`, `callsite_id:Some("cs")`.)

- [ ] **Step 2: Run — expect FAIL.**

```bash
cargo test -p al-call-hierarchy --lib db_effect_solver::tests::fixed_leaf -- --nocapture
```

- [ ] **Step 3: Implement `effective_sccs`:** filter `scc_entry.members` to those where
  `is_recomputed(m)`; build a `SccInputGraph` over exactly those nodes with `edges_by_from` projected
  to `to`-ids that are ALSO recomputed members (drop edges to leaves/missing/outside-SCC); call
  `tarjan_scc`; return its `.sccs`.

- [ ] **Step 4: Run — expect PASS.**

- [ ] **Step 5: Commit.**

```bash
git add src/engine/l4/db_effect_solver.rs src/engine/l4/mod.rs
git commit -m "feat(l4): effective-SCC re-decomposition (remove fixed leaves/missing, re-run Tarjan)"
```

---

### Task 4: PD product-graph reachability (Step A)

**Files:** `src/engine/l4/db_effect_solver.rs` (+ tests).

**Interfaces:**
```rust
/// A terminal (Known/Unknown) db-effect discovered during PD solving, keyed by base identity.
pub struct TerminalEmission { pub routine_id: String, pub base: (String,String,String), // (op,table,operation_id)
    pub temp: TempStateKind /* Known(_)|Unknown only */ }
/// A PD fact retained on a member: (base identity) with a caller-frame PD(index).
pub struct PdFact { pub routine_id: String, pub base: (String,String,String), pub param_index: u32 }

/// Solve ParameterDependent substitution as semi-naive reachability over
/// (base_effect_id, routine, PD(index)) product nodes, for ONE effective SCC.
/// Seeds = each member's base PD effects PLUS edge-substituted images of external-callee
/// (already-settled successor / fixed-leaf) PD effects at member out-edges.
/// PD->PD inserts a product node; PD->Known/Unknown emits a TerminalEmission and stops.
pub fn solve_pd_reachability(
    eff: &Scc,
    graph: &CombinedGraph,
    routines_by_id: &HashMap<String, &L3Routine>,
    settled: &HashMap<String, RoutineSummary>, // predecessor_final_map (successors + leaves)
    upgraded_bindings: &HashMap<String, Vec<UpgradedBinding>>,
) -> (Vec<PdFact>, Vec<TerminalEmission>);
```

Reuse the existing substitution logic — extract the body of `substitute_pd_temp_state`
(`summary_runner.rs:691`) into a shared `pub(crate)` fn if not already callable, so the solver and the
old `compose_routine` share ONE substitution implementation (DRY; guarantees parity).

- [ ] **Step 1: Write failing tests** covering PD→PD chain, PD→Known emission, PD→Unknown emission,
  external-successor PD seed, self-loop. Assert the returned `PdFact`/`TerminalEmission` sets equal
  hand-computed expectations for each tiny fixture.

- [ ] **Step 2: Run — expect FAIL.**

- [ ] **Step 3: Implement the semi-naive worklist.** State node = `(base, routine, param_index)`.
  Initialize from member base PD effects + substituted successor/leaf PD effects. Pop a state; for each
  intra-effective-SCC caller edge `caller -> this`, apply `substitute_pd_temp_state`:
  `PD(j)` → new state `(base, caller, j)` if unseen; `Known/Unknown` → `TerminalEmission`. Bound the
  visited set by observed `(base, routine, index)` triples (finite — do not assume `index <= #params`).

- [ ] **Step 4: Run — expect PASS.**

- [ ] **Step 5: Commit.**

```bash
git add src/engine/l4/db_effect_solver.rs src/engine/l4/summary_runner.rs
git commit -m "feat(l4): PD substitution as semi-naive product-graph reachability (Step A)"
```

---

### Task 5: Closed-form terminal union + per-member assembly (Steps B/C)

**Files:** `src/engine/l4/db_effect_solver.rs` (+ tests).

**Interfaces:**
```rust
/// Compute per-member db-effect PRESENCE sets for one effective SCC:
///   C = union(member terminal base effects) ∪ union(ACTUAL outgoing-edge successor terminal sets)
///       ∪ union(terminal emissions from Step A)
///   effects[v] = C ∪ member-v PD facts
/// Returns per-member EffectId bitsets (presence), plus the retained PD/terminal metadata
/// needed to materialize DbEffect later (temp_state per EffectId is encoded in the identity).
pub struct SccPresence { pub by_member: HashMap<String, Vec<u64>> /* bitset over universe */ }
pub fn closed_form_union(
    eff: &Scc, graph: &CombinedGraph,
    routines_by_id: &HashMap<String, &L3Routine>,
    settled: &HashMap<String, RoutineSummary>,
    base_summaries: &HashMap<String, RoutineSummary>,
    pd_facts: &[PdFact], terminal_emissions: &[TerminalEmission],
    universe: &mut EffectUniverse,
) -> SccPresence;
```

- [ ] **Step 1: Write failing test** — a recursive pair where all members must end with the identical
  terminal union, plus one member-specific PD fact:

```rust
#[test]
fn recursive_members_share_terminal_union_plus_own_pd() {
    // A<->B; A has base Known(true) effect e1; B has base Unknown effect e2; B has PD effect on b only.
    // Expect: A and B both contain {e1,e2}; only B additionally its PD-keyed effect.
    // (Build fixture; intern e1,e2; assert bitsets.)
}
```

- [ ] **Step 2: Run — expect FAIL.**

- [ ] **Step 3: Implement** `C` as a bitset union over interned terminal EffectIds (base + settled
  successor terminal effects reached over actual out-edges + Step-A terminal emissions); then set each
  member's bitset = `C | member_pd_bits(member)`.

- [ ] **Step 4: Run — expect PASS.**

- [ ] **Step 5: Commit.**

```bash
git add src/engine/l4/db_effect_solver.rs
git commit -m "feat(l4): closed-form terminal union + per-member presence (Steps B/C)"
```

---

### Task 6: `via` reconstruction (Step D)

**Files:** `src/engine/l4/db_effect_solver.rs` (+ tests).

**Interfaces:**
```rust
/// Reconstruct the per-(member, EffectId) via rank in one post-pass:
///   via(m,k) = max( base_via(m,k) if k∈base(m) [direct],
///                   via_for_edge_kind(e) for every ACTUAL edge e=(m,c) & k'∈set(c) with T_e(k')=k )
/// via NEVER propagates transitively (compose replaces it). Returns via string per (member,EffectId).
pub fn reconstruct_via(
    eff: &Scc, graph: &CombinedGraph, presence: &SccPresence,
    base_summaries: &HashMap<String, RoutineSummary>,
    settled: &HashMap<String, RoutineSummary>,
    universe: &EffectUniverse,
) -> HashMap<(String, EffectId), String>;
```

- [ ] **Step 1: Write failing test** — same effect reached via `direct` (rank 4) and `event-dispatch`
  (→ `event-subscriber`, rank 2): expect `direct`. Base effect → `direct`. Add a canonicalization
  assertion: an unknown via string must not silently win (rank 0 == `inherited`).

- [ ] **Step 2: Run — expect FAIL.**

- [ ] **Step 3: Implement** using `via_for_edge_kind` + `merge_via` semantics (max rank, first wins on
  tie). Init member ranks from base effects (`direct`), then fold each actual out-edge's rank after
  transforming the callee set.

- [ ] **Step 4: Run — expect PASS.**

- [ ] **Step 5: Commit.**

```bash
git add src/engine/l4/db_effect_solver.rs
git commit -m "feat(l4): via reconstruction post-pass (Step D)"
```

---

### Task 7: Side-facts solvers (uncertainties + has_unresolved_calls)

**Files:** `src/engine/l4/db_effect_solver.rs` (+ tests).

**Interfaces:**
```rust
/// has_unresolved_calls = boolean-OR reachability; uncertainties = per-effective-SCC set union
/// respecting the callsite-local kind filter (summary_runner.rs:443 skips member-not-found /
/// external-target / ambiguous-overload / interface-open-world from the INHERITED union; each
/// member still keeps its OWN such kinds + opaque-callee + uncertainty-edge kinds).
pub struct SideFacts {
    pub uncertainties: HashMap<String, Vec<Uncertainty>>, // per member, dedup+sorted by uncertainty_key
    pub has_unresolved: HashMap<String, bool>,
}
pub fn solve_side_facts(
    eff: &Scc, graph: &CombinedGraph, routines_by_id: &HashMap<String,&L3Routine>,
    settled: &HashMap<String,RoutineSummary>, base_summaries: &HashMap<String,RoutineSummary>,
    uncertainty_edges_by_from: &HashMap<String, Vec<usize>>,
) -> SideFacts;
```

- [ ] **Step 1: Write failing test** — a member with an inherited `member-not-found` (must be filtered
  from the union) vs an inherited generic kind (must propagate), plus an opaque-callee edge (must add
  `opaque-callee` + set `has_unresolved`). Compare against the old solver's output for the same fixture.

- [ ] **Step 2: Run — expect FAIL.**

- [ ] **Step 3: Implement** mirroring `compose_routine:442-501` exactly: same filter list, same
  `opaque-callee` construction, same `dedupe`/sort by `uncertainty_key`, same `has_unresolved` OR
  conditions (unresolved callee, opaque callee, uncertainty edges).

- [ ] **Step 4: Run — expect PASS.**

- [ ] **Step 5: Commit.**

```bash
git add src/engine/l4/db_effect_solver.rs
git commit -m "feat(l4): side-facts solvers (uncertainties union w/ filter, has_unresolved_calls OR)"
```

---

### Task 8: Assemble the solver into `compute_summaries_v2` + green the whole matrix

**Files:** `src/engine/l4/db_effect_solver.rs` (assembly fn), `src/engine/l4/summary_runner.rs`
(rewire `compute_summaries_v2_with_leaves` to use the solver for db_effects/uncertainties/
has_unresolved, keep `compose_routine`-derived `parameter_roles` + `in_recursive_cycle`).

**Interfaces:**
```rust
/// One-shot: build effective SCCs, solve PD, union, via, side-facts; materialize per-member
/// db_effects (sorted by (effect_key, operation_id) via universe.sorted_order()) with reconstructed
/// via + record_variable_id carried from the winning source effect (replicate old first-wins:
/// base last-write-wins, inherited collision keeps existing payload).
pub fn solve_scc_db_effects(scc_entry:&Scc, graph:&CombinedGraph,
    routines_by_id:&HashMap<String,&L3Routine>, settled:&HashMap<String,RoutineSummary>,
    base_summaries:&HashMap<String,RoutineSummary>, upgraded_bindings:&HashMap<String,Vec<UpgradedBinding>>,
    uncertainty_edges_by_from:&HashMap<String,Vec<usize>>, universe:&mut EffectUniverse,
    is_recomputed:&dyn Fn(&str)->bool)
  -> HashMap<String, (Vec<DbEffect>, Vec<Uncertainty>, bool)>;
```

- [ ] **Step 1: Rewire v2** — replicate `compute_summaries_with_leaves`'s scaffolding
  (`routines_by_id`, `base_summaries` for non-leaves, `uncertainty_edges_by_from`, leaf pre-seed,
  reverse-topo loop), but per SCC call `solve_scc_db_effects` for the db/uncertainty/has_unresolved
  triple and the existing `compose_routine` ONLY to harvest `parameter_roles` (and
  `in_recursive_cycle` from `scc_entry.recursive`). Assemble each member's `RoutineSummary`.

- [ ] **Step 2: Run the FULL differential matrix — iterate until every `parity_*` passes.**

```bash
cargo test -p al-call-hierarchy --test l4_summary_differential -- --nocapture
```
Expected: all `parity_*` green. Any mismatch prints the routine id + both summaries; fix the solver,
not the test.

- [ ] **Step 3: Run the L4/L5 lib tests** to catch any incidental breakage:

```bash
cargo test -p al-call-hierarchy --lib engine::l4 -- --nocapture
```
Expected: PASS.

- [ ] **Step 4: Commit.**

```bash
git add src/engine/l4/db_effect_solver.rs src/engine/l4/summary_runner.rs
git commit -m "feat(l4): assemble db_effect_solver into compute_summaries_v2; differential matrix green"
```

---

### Task 9: Real-corpus differential + record_variable_id proof

**Files:** `tests/l4_summary_differential.rs` (add a CDO/real-workspace-gated whole-program parity
test, mirroring the `scripts/cdo-gate` / `CDO_WS` pattern in `tests/common/cdo.rs`), plus a repo sweep.

- [ ] **Step 1: Add a `CDO_WS`-gated test** that assembles the real workspace, runs BOTH
  `compute_summaries_with_leaves` and v2 over the SAME `(routines, graph, scc, ub, fields, leaves)`
  built from the detector-context path, and asserts complete-`RoutineSummary` parity for every
  routine. Skip (no-op) when `CDO_WS` unset; panic when `ENFORCE_CDO_WS=1` and unset (use
  `tests/common/cdo.rs` helpers).

- [ ] **Step 2: Prove `record_variable_id` is out of contract** — grep and record in the test file's
  doc comment that `PDbEffect` (summary.rs:211) omits it and no reader exists:

```bash
rg -n "record_variable_id" src/engine/l5 src/engine/l4 | rg -v "None|test_support|: Option<String>"
```
Expected: no *reader* hits (only `None` constructors / the field decl). The complete-`RoutineSummary`
differential already guards the field regardless.

- [ ] **Step 3: Run against CDO** (user runs locally; document the command):

```bash
CDO_WS=<path> cargo test -p al-call-hierarchy --test l4_summary_differential cdo_ -- --nocapture
```
Expected: parity holds for all routines.

- [ ] **Step 4: Commit.**

```bash
git add tests/l4_summary_differential.rs
git commit -m "test(l4): CDO-gated whole-program v2 parity + record_variable_id out-of-contract proof"
```

---

### Task 10: Cutover — route production through v2, update trace oracle

**Files:** `src/engine/l4/summary.rs:795`, `capability_cone.rs:2100,2587`,
`detector_context.rs:510,771` (call v2), `summary_runner.rs` (retain old solver behind
`#[cfg(test)]`/flag as the differential oracle + trace-compat), any `RawSccTrace` oracle test.

- [ ] **Step 1: Point the four batch callers at v2.** For each, replace `compute_summaries*` with the
  v2 fn; where a caller needs `summarize_diagnostics`/`RawSccTrace`, have v2 return empty diagnostics
  (no cap in the closed-form path) and no trace. Keep the OLD fn compiled for the differential harness.

- [ ] **Step 2: Update/retire the trace-oracle test** (`summary.rs` `run_and_project` +
  `differential`): assert FINAL-summary semantics, not the 58-pass trajectory. If a golden encodes the
  trajectory, regenerate it to the final-semantics form and inspect the diff.

- [ ] **Step 3: Run the affected umbrellas + goldens.**

```bash
cargo test -p al-call-hierarchy --lib && bash scripts/check-goldens 2>&1 | tee /tmp/goldens.log; grep -i fail /tmp/goldens.log || echo GOLDENS_OK
```
Expected: lib PASS; `GOLDENS_OK`.

- [ ] **Step 4: Commit.**

```bash
git add -p   # stage only the intended files, review each hunk
git commit -m "feat(l4): route batch callers through v2 db-effect solver; trace oracle -> final semantics"
```

---

### Task 11: Measure on 8020 + perf gate

**Files:** `tests/perf_bounds.rs` (add a `compute_summaries` bound), `docs/` re-measure note.

- [ ] **Step 1: Re-run the 8020 probe** (detached, per the established pattern in
  `logs/run-probe.ps1`) with `ALSEM_TRACE=1 ALSEM_TRACE_DETAIL=hot`, capturing the
  `context.compute_summaries` span. Record before (924s) vs after.
- [ ] **Step 2: Roles-only re-measure** — confirm `walk_param` (`summary_runner.rs:625`) is not the
  new bottleneck; capture the `compute_summaries` residual breakdown.
- [ ] **Step 3: Add a perf bound** in `tests/perf_bounds.rs` asserting `compute_summaries` on the
  synthetic corpus stays within an order of magnitude of the new baseline (release-only, matching the
  file's existing gate style).
- [ ] **Step 4: Commit.**

```bash
git add tests/perf_bounds.rs
git commit -m "perf(l4): compute_summaries order-of-magnitude gate + 8020 re-measure"
```

---

### Task 12: Compact representation (memory) — optional within Phase 1

**Files:** `src/engine/l4/db_effect_solver.rs`, the projection path (`summary.rs:545`,
`capability_cone.rs:2650`).

**Rationale:** producing `RoutineSummary` once (not 58×) already removes the ~40 GB churn; this task
targets the residual ~2–3 GB of materialized `Vec<DbEffect>`. Since NO detector reads internal
`db_effects`, keep `RoutineSummary` as-is but have the solver hold presence as bitsets and materialize
`Vec<DbEffect>` lazily where the projection consumes it. Gate on a memory re-measure — skip if Task 11
already shows sub-GB.

- [ ] **Step 1:** measure peak RSS after Task 10. If already sub-GB, mark this task N/A in the ledger
  and skip. Else:
- [ ] **Step 2:** introduce a `CompactEffect { effect_id: EffectId, temp: TempStateKind, via_rank: u8,
  record_variable_id: Option<String> }` per-member store; materialize `DbEffect` via
  `universe.effect_key` + `sorted_order` only at the projection boundary.
- [ ] **Step 3:** re-run the differential matrix + CDO parity — must stay byte-identical.
- [ ] **Step 4: Commit** (or record N/A).

---

### Task 13 (DEFERRED within Phase 1): Salsa incremental path

The Salsa per-SCC path (`incremental/queries.rs:728`) calls `run_one_scc` directly and is LSP-only,
not on the 8020 batch path. Integrating the effective-SCC re-decomposition there interacts with SCC
cache keys — a separate design. Track as a follow-up; the LSP path keeps the old solver until then.
(Note in `docs/OUTSTANDING.md`.)

---

## Phase 2 — parameter_roles monotonization + cap (GATED ROADMAP — re-plan after Phase 1)

**Do not start until Phase 1 is merged and re-measured.** This is a SEPARATELY APPROVED soundness
change (may rebaseline a few role facts). Task outline (expand into bite-sized steps at planning time,
after the Phase-1 re-measure shows whether roles are even a bottleneck):

1. Extract & freeze the monotone c1b may-facts first (`persists_current_record`, `validates_param`,
   `copies_into_param`, …) — pure joins.
2. Property-test whether the residual path-summary transfer is monotone under the existing flat
   domains (`Loaded`, `Dirty` via `join_dirty:105`, `LoadedFields`). Do NOT reorder `Dirty` (its join
   is already a valid semilattice) and do NOT join `current_loaded_fields` (a strong update).
3. If non-monotone, represent procedures as monotone abstract transformers / finite input→output
   relations; derive `RecordRoleSummary` after convergence.
4. Replace `MAX_FIXED_POINT_ITERATIONS` with a debug-only diagnostic + a proven worklist bound (≤
   global product-lattice height). `LOOP_BOUND=3` (`cfg_walker.rs:344`) is separate — leave it.
5. Interim safety if roles still iterate: full repeated-state cycle detection that reproduces the
   iteration-1000 output (not a first-repeat jump, which changes the cycle phase).
6. Rebaseline any changed goldens as an explicit, adjudicated soundness change.

---

## Phase 3 — capability_cones (GATED ROADMAP — profiling confirm first)

Profile `context.capability_cones` (50s) and confirm the `ConeFacts = BTreeMap<String>` +
min-distance-witness shape (`capability_cone.rs:1338,1342,1360`) before planning. The cone closure is
a shortest-path (tropical) closure, not plain reachability — design the interned condensation
accordingly; reconstruct the witness on demand. Expand into tasks after the confirm.

---

## Phase 4 — substrate cache (GATED ROADMAP — optional, decide after Phase 1-3 re-measure)

Content-address the L4 substrate (workspace-scoped interned universe + bitsets + rank data + roles +
cones) by a source-set hash; the cache entry carries/verifies its universe generation (compact `u32`
IDs are meaningless across generations). Keyed by BC base-app version + content hash. Only worth
planning if Phases 1–3 leave a warm-run cost worth caching.

---

## Freebie — `fresh_coverage` reuse (independent, any time)

`run_analyze_with_exit` (`src/engine/gate/run.rs:206`) runs a second full program resolve (58s) only
for coverage stats. Investigate reusing the L3 assembly / skipping when coverage is unconsumed.
Independent single task; gated on output being unaffected.

---

## Self-Review (author checklist — completed)

- **Spec coverage:** Steps 0–D, side-facts split, effective-SCC re-decomposition, interning
  (workspace-scoped + sorted materialization), via post-pass, record_variable_id proof + complete
  differential, compact repr, trace-oracle update, perf gate, Phase 1 strict / Phase 2 separate — each
  maps to a task (1–12). Salsa deferred (13). Phases 2–4 gated roadmaps per the spec's own deferrals.
- **Placeholders:** solver-core tasks give exact signatures + concrete test fixtures + the algorithm;
  bodies are TDD-discovered against the differential (appropriate for a from-scratch solver, not
  prose placeholders). Mechanical tasks (interner, harness, cutover) carry literal code.
- **Type consistency:** `EffectId`/`EffectIdentity`/`EffectUniverse`, `effective_sccs`,
  `solve_pd_reachability`→`PdFact`/`TerminalEmission`, `closed_form_union`→`SccPresence`,
  `reconstruct_via`, `solve_side_facts`→`SideFacts`, `solve_scc_db_effects`,
  `compute_summaries_v2_with_leaves` — names consistent across tasks; all consume the verbatim
  existing interfaces block.
