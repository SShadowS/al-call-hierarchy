# Call-Graph Resolution — Typed Receiver Model Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Use superpowers:systematic-debugging for any failure (root cause before fixes).

**Goal:** Drive the CDO real-`unknown` call-edge rate (23.6% today) down to its provably-dynamic residual via strict-enum taxonomy, a generated platform-global catalog, framework-catalog gaps, then typed receiver dispatch — without a false-positive explosion in the L5 detectors.

**Architecture:** Replace the stringly-typed `(dispatch_kind, resolution)` ladder with strict Rust enums (`DispatchKind` / `Resolution`, folding `UnknownReason` into `Resolution::Unknown(reason)`), keeping the golden/projection boundary byte-stable via `enum→&str`. Then ship pure RECLASSIFICATION (no new resolved edges): a generated bare-global catalog + framework-catalog gaps. Baseline the metric cleanly. THEN ship graph EXPANSION (new resolved edges) phase by phase — Record table-procedure dispatch, receiver-type scope/globals/return-type inference, TableID const-prop, guard-predicate suppression — each gated by a fresh CDO re-measurement + detector-precision re-triage.

**Tech Stack:** Rust, `phf` perfect-hash catalogs, tree-sitter-al V2, rayon. Goldens are Rust-owned (`REGEN_TEMP_GOLDENS=1 cargo test`). Measurement harness: `aldump --l3-call-graph-stats <ws>` + `aldump --l3-unknown-breakdown <ws>`. CDO app under test: `U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud`.

**Branch:** `engine-d22` (continue; off `master`).

---

## Ground truth (verified in source before planning)

- `CallEdge` (`src/engine/l3/call_resolver.rs:100`) carries `dispatch_kind: String`, `resolution: String`, `unknown_reason: Option<UnknownReason>`.
- **All `dispatch_kind` string values:** `direct`, `interface`, `builtin`, `unresolved`, `dynamic`, `method`, `implicit-trigger`, `page-run`, `report-run`, `codeunit-run`.
- **All `resolution` string values:** `resolved`, `maybe`, `unknown`, `builtin`, `member-not-found`, `ambiguous`, `opaque`, `external-target`.
- **`UnknownReason`** (`call_resolver.rs:55`): `BareUnresolved`, `CompoundReceiver`, `UntrackedReceiver`, `RecordTableProcedure`, `FrameworkMethodNotInCatalog`, `NonObjectReceiverType`, `EnumStatic`, `CalleeUnknown`, `InterfaceNoImpl`. Has a `.label()` → kebab string (used by `--l3-unknown-breakdown`).
- **String consumers** (must keep reading the same strings, via `.as_str()` after the refactor): `call_graph_projection.rs::project_edge` (golden boundary — `PCallEdge.dispatch_kind`/`.resolution` stay `String`), `resolution_class.rs::classify`, `coverage.rs::is_unresolved_resolution` + `dispatch_kind == "dynamic"` filter, `snapshot.rs::map_dispatch_kind` + `dispatch_kind == "implicit-trigger"`/`"dynamic"`, detectors `d2.rs`/`d38.rs`/`d47.rs`, `unresolved_cone.rs`, `root_classification.rs`, `event_flow.rs`, `ordering_inter.rs`, `ordering_engine.rs`, `combined_graph.rs`, `dep_artifact_l4.rs`.
- **The dynamic object-run edge** (`call_resolver.rs:493`) emits `dispatch_kind="dynamic"`, `resolution="unknown"`, NO reason. `classify()` buckets it `Dynamic` via `dispatch_kind` BEFORE reading `resolution`, so it is NOT counted as a true unknown and is excluded from the breakdown. The enum refactor must preserve this: it needs a reason to satisfy "every `Resolution::Unknown` carries a reason," so add `UnknownReason::DynamicObjectRunTarget` (stringifies under `resolution="unknown"`, still buckets `Dynamic`).
- **Inline fixture test harness:** `assemble_and_resolve_default(&[(String,String)], app_guid).project_call_graph()` → `L3CallGraphProjection { groups: Vec<PCallsiteGroup{ edges: Vec<PCallEdge> }> }`. Walk `groups.iter().flat_map(|g| g.edges.iter())`. See `tests/l3cg_member_builtins.rs`.
- **Catalogs:** `al_builtins.rs::global_builtin_disposition(name) -> Option<&'static str>` (~25 entries, bare path). `member_builtins.rs::{classify_receiver, member_builtin_disposition}` (`phf`, member path).
- **Symbol table:** `symbol_table.rs::routines_in_object(object_id)`, `routine_in_object(object_id, name)`, `object_by_type_name`, `object_by_type_number`.

---

## Task 1: Strict taxonomy enums (internal-only refactor, goldens byte-stable)

**Files:**
- Create: `src/engine/l3/taxonomy.rs` — `DispatchKind`, `Resolution` enums + `as_str()` + `FromStr`/parse helpers.
- Modify: `src/engine/l3/call_resolver.rs` — `CallEdge.dispatch_kind: DispatchKind`, `.resolution: Resolution` (folds `unknown_reason`); update every assignment + `CallEdge::base`.
- Modify: `src/engine/l3/mod.rs` — `pub mod taxonomy;`.
- Modify: every consumer that reads `.dispatch_kind`/`.resolution` as `&str` → call `.as_str()` (or match the enum). Files listed in Ground Truth.
- Modify: `src/engine/l3/resolution_class.rs::classify` + `unknown_breakdown` to consume enums.
- Test: `src/engine/l3/taxonomy.rs` (unit `#[cfg(test)]`), and the FULL existing suite must stay green with ZERO golden changes.

**Design — the enums (exact variants & strings):**

```rust
// src/engine/l3/taxonomy.rs
use super::call_resolver::UnknownReason;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchKind {
    Direct, Interface, Builtin, Unresolved, Dynamic, Method,
    ImplicitTrigger, PageRun, ReportRun, CodeunitRun,
}

impl DispatchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DispatchKind::Direct => "direct",
            DispatchKind::Interface => "interface",
            DispatchKind::Builtin => "builtin",
            DispatchKind::Unresolved => "unresolved",
            DispatchKind::Dynamic => "dynamic",
            DispatchKind::Method => "method",
            DispatchKind::ImplicitTrigger => "implicit-trigger",
            DispatchKind::PageRun => "page-run",
            DispatchKind::ReportRun => "report-run",
            DispatchKind::CodeunitRun => "codeunit-run",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Resolved,
    Maybe,
    Builtin,
    MemberNotFound,
    Ambiguous,
    Opaque,
    ExternalTarget,
    Unknown(UnknownReason),
}

impl Resolution {
    pub fn as_str(self) -> &'static str {
        match self {
            Resolution::Resolved => "resolved",
            Resolution::Maybe => "maybe",
            Resolution::Builtin => "builtin",
            Resolution::MemberNotFound => "member-not-found",
            Resolution::Ambiguous => "ambiguous",
            Resolution::Opaque => "opaque",
            Resolution::ExternalTarget => "external-target",
            Resolution::Unknown(_) => "unknown",
        }
    }
    /// The folded cause, present iff this is `Unknown`.
    pub fn unknown_reason(self) -> Option<UnknownReason> {
        match self { Resolution::Unknown(r) => Some(r), _ => None }
    }
}
```

`CallEdge::base` initializes `dispatch_kind: DispatchKind::Unresolved`, `resolution: Resolution::Unknown(UnknownReason::CalleeUnknown)` (a safe fail-closed default that every emission site overwrites). Remove the separate `unknown_reason` field — `Resolution::Unknown(reason)` is the single source. Add `UnknownReason::DynamicObjectRunTarget`.

- [ ] **Step 1: Write the failing test** (`src/engine/l3/taxonomy.rs` test module)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::l3::call_resolver::UnknownReason;

    #[test]
    fn dispatch_kind_strings_are_golden_stable() {
        for (dk, s) in [
            (DispatchKind::Direct, "direct"),
            (DispatchKind::Interface, "interface"),
            (DispatchKind::Builtin, "builtin"),
            (DispatchKind::Unresolved, "unresolved"),
            (DispatchKind::Dynamic, "dynamic"),
            (DispatchKind::Method, "method"),
            (DispatchKind::ImplicitTrigger, "implicit-trigger"),
            (DispatchKind::PageRun, "page-run"),
            (DispatchKind::ReportRun, "report-run"),
            (DispatchKind::CodeunitRun, "codeunit-run"),
        ] {
            assert_eq!(dk.as_str(), s);
        }
    }

    #[test]
    fn resolution_strings_are_golden_stable_and_unknown_folds_reason() {
        assert_eq!(Resolution::Resolved.as_str(), "resolved");
        assert_eq!(Resolution::Maybe.as_str(), "maybe");
        assert_eq!(Resolution::Builtin.as_str(), "builtin");
        assert_eq!(Resolution::MemberNotFound.as_str(), "member-not-found");
        assert_eq!(Resolution::Ambiguous.as_str(), "ambiguous");
        assert_eq!(Resolution::Opaque.as_str(), "opaque");
        assert_eq!(Resolution::ExternalTarget.as_str(), "external-target");
        let u = Resolution::Unknown(UnknownReason::BareUnresolved);
        assert_eq!(u.as_str(), "unknown");
        assert_eq!(u.unknown_reason(), Some(UnknownReason::BareUnresolved));
        assert_eq!(Resolution::Resolved.unknown_reason(), None);
    }
}
```

- [ ] **Step 2: Run test to verify it fails** — `cargo test -p al-call-hierarchy taxonomy::tests` → FAIL (module/types not defined).
- [ ] **Step 3: Implement** `taxonomy.rs` as above; add `pub mod taxonomy;` to `src/engine/l3/mod.rs`; add `UnknownReason::DynamicObjectRunTarget` + its `.label()` arm (`"dynamic-objectrun-target"`).
- [ ] **Step 4: Refactor `CallEdge`** — change field types to `DispatchKind`/`Resolution`, delete `unknown_reason` field. Update `CallEdge::base`. At every emission site in `call_resolver.rs` set the enum (e.g. `e.dispatch_kind = DispatchKind::Direct; e.resolution = Resolution::Resolved;`; unknown sites `Resolution::Unknown(UnknownReason::X)`; the dynamic object-run site → `DispatchKind::Dynamic` + `Resolution::Unknown(UnknownReason::DynamicObjectRunTarget)`). `object_run_dispatch_kind` returns `DispatchKind`. `implicit_edges.rs` returns enums.
- [ ] **Step 5: Update consumers** — at each `.dispatch_kind`/`.resolution` read site, append `.as_str()` (string compares unchanged) OR convert to an enum match where cleaner. `resolution_class.rs::classify` takes `(Resolution, DispatchKind)`; `unknown_breakdown` reads `e.resolution.unknown_reason()` instead of `e.unknown_reason`. `project_edge` emits `e.dispatch_kind.as_str().to_string()` / `e.resolution.as_str().to_string()`.
- [ ] **Step 6: Build** — `cargo build --lib` → clean. Fix every match-exhaustiveness error the compiler surfaces (this is the point of the enum: it finds every site).
- [ ] **Step 7: Verify goldens byte-stable** — `cargo test` (full). Expected: GREEN, ZERO golden diffs. If any golden changed, a string mapping is wrong — fix the `as_str` arm, do NOT regen.
- [ ] **Step 8: Commit** — `git add src/engine/l3/taxonomy.rs src/engine/l3/mod.rs src/engine/l3/call_resolver.rs src/engine/l3/resolution_class.rs src/engine/l3/implicit_edges.rs <touched consumers>` then `git commit -m "refactor(engine-d22): strict DispatchKind/Resolution enums; fold UnknownReason into Resolution::Unknown"`. Update `CHANGELOG.md` (Changed).

**Contract oracle to add** (`tests/l3cg_oracles.rs`): assert that for every projected edge with `resolution == "unknown"`, walking the resolver edge it must carry a non-`DynamicObjectRunTarget` reason iff it classifies as a true `Unknown` — i.e. "unattributed is structurally impossible." (If the oracle layer only sees projected strings, instead assert via a resolver-level test that no `CallEdge` with `Resolution::Unknown` can be constructed without a reason — which the enum guarantees by construction; document this and keep the existing `unattributed == 0` breakdown oracle.)

---

## Task 2: Generated bare-global catalog (reclassify ~1247 `bare-unresolved`)

**Rationale:** AL forbids unqualified cross-object calls, so a bare `Foo()` not in the caller's own object and not in the ~25-entry allowlist is overwhelmingly a missing PLATFORM GLOBAL (GuiAllowed, CreateInStream, StrSubstNo overloads, Database::, etc.). A hand-list drifts per BC version → wrong architecture. Build an OFFLINE generator that emits a `phf_set!`/`phf_map!` from an authoritative source; apply on the bare `NotFound` path as pure reclassification (NO new resolved-to-routine edges).

**Files:**
- Create: `tools/gen-al-builtins/` (a checked-in generator — a small Rust `bin` or script). Source priority: (1) dump `Microsoft.Dynamics.Nav.CodeAnalysis.dll` symbols if present on the machine; (2) else scrape the MS Learn AL method reference (cached HTML vendored into the repo once). Emits `src/engine/l3/global_builtins.rs` with a provenance header (BC version + source + date).
- Create: `src/engine/l3/global_builtins.rs` (GENERATED) — `pub fn global_builtin_disposition_generated(name_lc: &str) -> Option<GlobalBuiltinClass>` backed by `phf`.
- Modify: `src/engine/l3/al_builtins.rs` — `global_builtin_disposition` consults the generated set after the hand allowlist (hand list stays as the curated effectful/control-terminating overlay).
- Modify: `src/engine/l3/call_resolver.rs` — the `PCallee::Bare` `NotFound` branch already calls `global_builtin_disposition`; no logic change beyond the catalog now being larger. Confirm the `BareUnresolved` reason only fires on a true miss.
- Modify: `src/engine/l3/mod.rs` — `pub mod global_builtins;`.
- Test: `tests/global_builtins_catalog.rs` (inline-AL fixtures) + extend `tests/l3cg_oracles.rs`.

**Decision gate (product fork — surface to user only if the authoritative source is unavailable):** if neither the DLL nor a scrapeable/vendorable MS Learn list is obtainable on this machine, STOP and ask. Otherwise proceed with whichever source is available and record provenance.

- [ ] **Step 1: Write the failing test** — fixture: a codeunit calling a known platform global that is NOT in the hand allowlist (e.g. `GuiAllowed()`), assert the edge resolves `builtin` not `unknown`.

```rust
// tests/global_builtins_catalog.rs
use al_call_hierarchy::engine::l3::l3_workspace::assemble_and_resolve_default;
const APP_GUID: &str = "2b000000-0000-0000-0000-0000000002bb";

#[test]
fn platform_global_not_in_hand_allowlist_is_builtin() {
    let src = "codeunit 50200 A { procedure Go() begin if GuiAllowed() then Message('x'); end; }";
    let owned = vec![("src/a.al".to_string(), src.to_string())];
    let p = assemble_and_resolve_default(&owned, APP_GUID).project_call_graph();
    let edges: Vec<_> = p.groups.iter().flat_map(|g| g.edges.iter()).collect();
    let guiallowed = edges.iter().find(|e| e.operation_id.contains("op")); // refine: locate the GuiAllowed callsite
    assert!(
        edges.iter().any(|e| e.dispatch_kind == "builtin" && e.resolution == "builtin"),
        "GuiAllowed must be a builtin edge; got {:#?}", edges
    );
    assert_eq!(
        edges.iter().filter(|e| e.resolution == "unknown").count(), 0,
        "no platform-global bare call stays unknown"
    );
}
```

- [ ] **Step 2: Run** → FAIL (GuiAllowed currently `unknown` / `bare-unresolved`).
- [ ] **Step 3: Build the generator** — `tools/gen-al-builtins/main.rs`: parse the chosen source into a sorted, deduped, lowercased name list (+ overload arity where the source carries it); emit `global_builtins.rs` via `phf_codegen` (or a literal `phf_set!`/`phf_map!` text emit). Header comment: provenance. Run it; commit the generated file + generator + vendored source snapshot.
- [ ] **Step 4: Wire** `global_builtin_disposition` to consult generated set after the hand list. Rebuild `cargo build --lib`.
- [ ] **Step 5: Run** the new test → PASS. Run `cargo test l3cg` → existing member/global tests green.
- [ ] **Step 6: Re-measure CDO** — build `cargo build --release --bin aldump`; run `--l3-call-graph-stats` + `--l3-unknown-breakdown`. Confirm `bare-unresolved` dropped by ~1247 (to its irreducible residual = genuine typos / truly-missing), realUnknownRate fell, and `resolved` count UNCHANGED (pure reclassification). Record numbers in the commit body.
- [ ] **Step 7: Reconsider regenerating `member_builtins.rs`** from the same generator so both catalogs share one source of truth. If the source cleanly covers Record/RecordRef/framework intrinsics, regenerate `member_builtins.rs` and diff against the hand catalog — any DROP is a regression to investigate (the hand list may have entries the source lacks); any ADD is a gain. Keep whichever is the superset; document the merge.
- [ ] **Step 8: Commit** — generator + generated catalog(s) + wiring + tests + CHANGELOG (Added). Message records BC-version provenance + CDO delta.

---

## Task 3: Framework catalog gaps (reclassify ~39 `framework-method-not-in-catalog`)

**Files:**
- Modify: `src/engine/l3/call_resolver.rs` — extend `--l3-unknown-breakdown` to NAME the missing `(kind, method)` pairs (so the gap list is concrete). Add an `aldump --l3-framework-gaps <ws>` sub-report OR augment the existing breakdown output with the top missing `(ReceiverBuiltinKind, method)` tuples.
- Modify: `src/engine/l3/member_builtins.rs` — add the named-missing methods to the relevant `phf` sets.
- Test: `tests/l3cg_member_builtins.rs` (add cases) + the gap report.

- [ ] **Step 1:** Run `aldump --l3-unknown-breakdown` on CDO with the new naming → get the concrete `(kind, method)` gap list.
- [ ] **Step 2: Write failing tests** — one inline fixture per newly-named framework method asserting `builtin`.
- [ ] **Step 3:** Verify each named method against the MS Learn reference (or the generator source) — only add GENUINE intrinsics; a misspelled/user method must NOT be added (it would mask a real `unknown`).
- [ ] **Step 4:** Add verified methods to `member_builtins.rs` sets. `cargo test l3cg_member_builtins` → PASS.
- [ ] **Step 5: Re-measure CDO** — `framework-method-not-in-catalog` → ~0; resolved unchanged.
- [ ] **Step 6: Commit** — CHANGELOG (Added) + CDO delta.

---

## Task 4: Clean reclassification baseline (gate before graph expansion)

**Files:** measurement only + oracle hardening.

- [ ] **Step 1:** Build release `aldump` + `alsem`. Run `--l3-call-graph-stats` + `--l3-unknown-breakdown` on CDO. Confirm cumulative: unknown dropped ~1286 (1247 + 39), realUnknownRate materially down, ZERO new resolved edges across Tasks 1–3, and the breakdown now dominated by the EXPANSION buckets (`record-table-procedure` ~812, `untracked-receiver` ~881, `compound-receiver` ~243).
- [ ] **Step 2: Detector-precision check** — run `alsem analyze` on CDO; confirm the actionable-finding count + per-detector TP% did NOT move (Tasks 1–3 add NO edges, so detectors must be byte-stable). Any movement = a reclassification leaked into the cone; root-cause it.
- [ ] **Step 3: Extend contract oracles** (`tests/l3cg_oracles.rs`): every `builtin` edge's method is in a catalog (global or member); no edge is both `builtin` and `resolved`; every `unknown` carries a concrete `UnknownReason`; every `resolved` edge's `to` exists in the symbol table.
- [ ] **Step 4: Commit** the baseline as a labeled checkpoint. `cargo test` full → GREEN. Message: "clean reclassification baseline — CDO realUnknownRate X%".

---

## Task 5: Phase 3 — Record table-procedure dispatch (first NEW resolved edges, ~812)

> **GATE:** Re-measure CDO BEFORE starting (capture the `record-table-procedure` count). This task lands NEW resolved edges → pair with a CDO detector re-triage (spec §7) on completion.

**Architecture:** Implement the spec's `ReceiverType` lattice + Phase-A/B typed dispatch. Route the EXISTING object path through it FIRST (goldens stable — proves the refactor sound), THEN add Record-receiver dispatch: a Record method NOT in the builtin catalog resolves via `routines_in_object(tableObj)` using the receiver's effective table (reuse the d22 `record_types` effective-own-table logic).

**Files:**
- Create: `src/engine/l3/receiver_type.rs` — `ReceiverType` lattice (`Object{kind,id}`, `Record{table_id}`, `RecordRef`/`FieldRef`/`KeyRef`, `Interface{name}`, `Framework{kind}`, `Enum{name}`, `Primitive`, `SystemSingleton`, `Unknown`) + `infer_receiver_type(receiver, env) -> ReceiverType` (Phase A) + `dispatch(rt, method, arity, env) -> Resolution-producing outcome` (Phase B).
- Modify: `src/engine/l3/call_resolver.rs::resolve_call_site` `PCallee::Member` — replace the Step-3 ladder with `infer_receiver_type` → `dispatch`. Object/interface/enum branches must produce IDENTICAL edges to today (regression proof).
- Modify: `symbol_table.rs` if a `table_id → object_id` lookup is missing.
- Test: `tests/l3cg_record_dispatch.rs` (inline fixtures: a table with a user procedure called via `Rec.Proc()` resolves; a Record builtin still `builtin`; a missing method `member-not-found`).

**Sub-steps (TDD):**
- [ ] **Step 1:** Failing test — table 50000 with `procedure CalcDiscount()`, codeunit doing `Cust.CalcDiscount()` where `Cust: Record Customer` → assert edge `resolution == "resolved"`, `to` is the table procedure.
- [ ] **Step 2:** Build `ReceiverType` + `infer_receiver_type` for the EXISTING cases first (Object/Interface/Enum/Framework/Record-builtin), routing the current logic through it. Full suite GREEN with zero golden diff = refactor sound.
- [ ] **Step 3:** Add Record-receiver Phase-B branch (b): catalog miss → `routines_in_object(tableObj)` arity dispatch → `resolved`/`member-not-found`. Use the d22 effective-own-table logic for the receiver's table id.
- [ ] **Step 4:** New test → PASS. Full `cargo test` → triage every golden diff (some `record-table-procedure` unknowns legitimately become `resolved`/`method` edges — regen Rust-owned goldens, inspect each diff is intended).
- [ ] **Step 5: Re-measure CDO** — `record-table-procedure` → near 0; `resolved` UP ~800; realUnknownRate down.
- [ ] **Step 6: Detector re-triage** (spec §7) — `alsem analyze` CDO; use the `triage-findings` skill. Per-detector TP% must NOT regress. New transitive FPs (e.g. `db-op-in-loop` now tracing through a resolved table proc) are the signal for the Task 8 guard work — record them, do not suppress ad hoc.
- [ ] **Step 7: Commit** — CHANGELOG (Added/Changed) + CDO delta + triage summary.

---

## Task 6: Phase 4 — receiver-type scope + globals + return-type + chained inference (untracked ~881 + compound ~243)

> **GATE:** Re-measure CDO before/after. Lands new resolved edges → detector re-triage.

**Architecture (spec §3 + §2 Phase A recursion):** Persist a receiver-type environment L2→L3 (prefer option (a): additive per-callsite metadata — WITH-receiver var name + implicit-`Rec` flag, mirroring how `loop_stack`/`in_until_condition` were threaded). The env the resolver consults: **Locals → Params → Globals → implicit `Rec` → ordered enclosing `WITH` receivers.** Add object-global var resolution, `CurrPage`/`CurrReport` host typing, and method/return-type inference for chained `a.b.M()` and `GetX().M()` (cap recursion ~3, like L4 `MAX_CHASE_DEPTH`).

**Files:**
- Modify: `src/engine/l2/body_walk.rs` + `src/engine/l2/features.rs` — thread per-callsite receiver-scope metadata (WITH receiver, implicit-Rec).
- Modify: `src/engine/l3/receiver_type.rs::infer_receiver_type` — consult the persisted env; add Globals, `CurrPage`/`CurrReport` singletons, return-type chaining.
- Modify: `src/engine/l3/l3_workspace.rs` if the L2→L3 assembly must carry the new metadata.
- Test: `tests/l3cg_receiver_scope.rs` — WITH-block call, implicit-Rec trigger call, object-global call, `CurrPage.Update()`, chained `GetCust().Name()`.

**Sub-steps:** standard TDD per case (failing test → thread metadata → infer → resolve → re-measure). Decompose into one sub-task per inference source (WITH, implicit-Rec, globals, CurrPage/CurrReport, return-type chain) so each lands + re-measures independently. Cap chained recursion at depth 3. Re-triage CDO after the batch.

---

## Task 7: Phase 5 — intra-procedural TableID/Enum const-prop (dynamic→static; reclassify ~70 floor)

> **GATE:** Re-measure. Keep layers ACYCLIC — no L4→L3 feedback.

**Architecture (spec §5):** Cheap intra-procedural const tracker in L3 Phase A. `MyId := Database::Customer` caches a static table id in the routine env; `RecordRef.Open(MyId)` / `.GetTable(rec)` flows a static `Record{table_id}` (dynamic→resolved). Cross-procedural / DB-derived ids stay honestly `dynamic` (a new `Resolution` lens — NOT `unknown`). Reclassify the `non-object-receiver-type` Variant/primitive floor (~70) to `dynamic` where genuinely runtime-typed.

**Files:**
- Modify: `src/engine/l3/receiver_type.rs` — add the intra-procedural const tracker; `RecordRef` with a known table id → `Record{table_id}` dispatch.
- Modify: `src/engine/l3/taxonomy.rs` — if a distinct `dynamic` resolution string is needed at the projection boundary, add it (and update consumers + goldens together; we own them).
- Test: `tests/l3cg_const_prop.rs` — `RecordRef.Open(Database::Customer)` then `.Field(...)` flows the table; a parameter-derived id stays `dynamic`.

**Sub-steps:** TDD per case. After: the `non-object-receiver-type` + RecordRef dynamic buckets should be honestly `dynamic`, not `unknown`. Re-measure + re-triage.

---

## Task 8: Phase 6 — guard-predicate edge annotations + L5 guard-intersection suppression

> **GATE:** This is the FP-control lever for all new edges from Tasks 5–7. Re-measure detector precision before/after.

**Architecture (spec §7):** At L2/L3 tag each call/op edge with a minimal high-impact guard set: `GuiAllowed`, `IsTemporary`, `HasFilter` (extend as needed). CRITICAL: walk CFG BLOCK DOMINANCE (not just AST nesting) and carry POLARITY — AL leans on early-exit guards (`if not GuiAllowed then exit;` then the call → tag `GuiAllowed:false` is impossible past that point, the call requires `GuiAllowed:true`... encode the dominating-guard polarity precisely). Cone stays flow-insensitive (fast set-union). At L5, detectors intersect guard tags along the reachability path and discard findings whose path requires a guard incompatible with the root context (e.g. requires `GuiAllowed:true` but root is a background session, or a path crosses an edge tagged incompatibly).

**Files:**
- Modify: `src/engine/l2/body_walk.rs` — compute dominating-guard set + polarity per call/op edge.
- Modify: `src/engine/l2/features.rs` / edge model — carry guard tags.
- Modify: `src/engine/l5/*` (the cone + relevant detectors) — guard-intersection suppression at finding time.
- Test: `tests/l5_guard_suppression.rs` — a `GuiAllowed`-guarded call under a background-session root is suppressed; an unguarded one is not.

**Sub-steps:** TDD. Build the guard model first (dominance + polarity), then the L5 consumption. Re-triage CDO — confirm the Task 5–7 FP wave is suppressed AND no true positive is lost (the precision lever, measured).

---

## Final validation (after Task 8)

- [ ] `cargo build --release --bin aldump` + `--bin alsem`. Final CDO `--l3-call-graph-stats` + `--l3-unknown-breakdown`: report the full before/after journey (23.6% → residual), with the residual provably dynamic.
- [ ] Full `cargo test` GREEN. `KNOWN_DIVERGENCES.json` stays `[]`.
- [ ] All contract oracles in `tests/l3cg_oracles.rs` pass (resolved→symbol-exists, builtin→catalog, unknown→reason, dynamic→genuinely-runtime, no builtin∧resolved).
- [ ] Per-detector TP% on CDO non-regressed across all phases (the §7 guard work is the lever).
- [ ] Rust-owned goldens rebaselined via `REGEN_TEMP_GOLDENS=1`; every diff inspected as intended (CRLF/EOL churn confirmed 0/0 via `git diff --numstat`). Manifest "matrix" oracles updated to current Rust totals.
- [ ] CHANGELOG.md updated per task. Commits in logical groups.
- [ ] Secret-scan the full diff. THEN fast-forward `master` to the branch in the main worktree (`git -C U:\Git\al-call-hierarchy merge --ff-only engine-d22`) + `git -C U:\Git\al-call-hierarchy push origin master`.

---

## House rules (binding for every task)

- `rustfmt <file>` per file — NEVER `cargo fmt`. Stage only intended paths — NEVER `git add -A`.
- Goldens Rust-owned; regen ONLY via `REGEN_TEMP_GOLDENS=1`. NEVER read/write `U:\Git\al-sem` at test time.
- Engine NEVER panics — every new path fails closed to `Resolution::Unknown(reason)` / a conservative default. Additive to `src/engine/*`.
- DISK on U: is tight (NTFS dedup/VSS lag). `cargo clean` between heavy build cycles; a full debug `cargo test` build is ~30 GB — keep headroom (verify `Get-PSDrive U`).
- Git bash + Windows paths; no `2>nul`.
- Verify (`cargo build --lib` + targeted tests) WITHIN each turn before any long full-suite run.
```
