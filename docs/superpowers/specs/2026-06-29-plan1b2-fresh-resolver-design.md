# Plan 1B.2 — Fresh Call Resolver over ProgramGraph (clean-room, dual-run gated)

> Status: DESIGN (pre-plan), **v2** — revised after adversarial review by gpt-5.5 (verdict:
> NO-GO-as-written) and gemini-3.1-pro (GO-WITH-CHANGES). Both caught the same class of flaw
> that the charter review caught (an under-factored edge model + a fragile oracle). v2 repairs
> the edge contract, the evidence semantics, the metric, and the differential oracle before any
> code is written. Serves the [BC Semantic-Intelligence Charter](2026-06-28-bc-semantic-intelligence-charter.md).
> Builds on Plan 1B.1 (`src/program/` — app-qualified NodeId, dependency topology, topology-scoped object index).
> Successor: Plan 1B.3 (full ABI cross-check + deep re-baseline + retire L3).

## 1. Goal

Build a **fresh, clean-room call/behaviour-edge resolver** over the `ProgramGraph` substrate that:

1. Walks routine bodies over the al-syntax owned IR and extracts call sites + synthetic
   behaviour sites (data-is-control-flow triggers, object runs, event flow).
2. Resolves each target **scoped to the correct world** — name lookup within the caller app's
   dependency closure; fan-out (interface/event/extension) over the analyzed snapshot with an
   explicit open-world tail — never flat-global, never silently closed-world.
3. Emits **multi-axis edges** `(EdgeKind, DispatchShape, SetCompleteness, Vec<Route>)` where each
   `Route` carries its own `Evidence` and optional `Condition`, so one site can fan out to targets
   of mixed provenance, visibility, and runtime-conditionality.
4. Is validated at every step by a **dual-run differential oracle** against the existing L3
   resolver — but only for the edge domains L3 actually models, aligned by stable semantic site
   identity (not ordinals) — so the from-scratch rebuild provably matches-or-beats the current
   0.11% real-unknown baseline on the CDO corpus before anything ships.

This is a **clean-room rebuild**: the resolver, the receiver-type inference, the edge model, AND
the builtin catalogs are re-derived from their authoritative sources, not grandfathered from L3.
The existing L3 resolver and catalogs are kept **only as oracle/validators**, never seeded.

### Non-goals (this spec)

- The **full** ABI cross-check against `.app` SymbolReference and retiring the L3 oracle — Plan 1B.3.
  (v2 DOES pull a *minimal* witness requirement forward — §5.5 — so false `Source`/`Abi` certainty
  cannot become the new baseline; the heavyweight SymbolReference reconciliation stays 1B.3.)
- Replacing the LSP handlers' current data source — fresh runs alongside (flag / harness) until 1B.3.
- Change-impact queries (charter Spec 4).

## 2. Decision record (brainstorm 2026-06-29)

Four decisions taken with the user, each the more ambitious option, made safe by the oracle.
**These are settled — v2 critiques and repairs the design that implements them, not the choices.**

| # | Decision | Chosen | Rationale |
|---|----------|--------|-----------|
| 1 | Reuse L3 app-scoped vs build fresh | **Build fresh over ProgramGraph** | Best long-term architecture; ProgramGraph-native; refactoring on the table pre-release. |
| 2 | Validation strategy | **Dual-run differential oracle** | "Dual-run parity is the spine." Keep L3 as a non-shipping oracle; gate every slice against it. |
| 3 | Keep vs re-derive within "fresh" | **Re-derive everything from zero** | Clean-room resolver, type inference, catalogs. Authoritative sources + oracle validate. |
| 4 | Spec granularity | **One mega-spec** (this doc), internally harness-first | Single context; the phases below are sequenced but live in one spec/plan. |

## 3. The multi-axis Edge model (the shared contract)

The central data structure. v1 used `(DispatchShape, Vec<Route>)`; review showed that is
under-factored. v2 separates four orthogonal questions every behaviour edge must answer:

- **What relation is this?** → `EdgeKind`
- **How is the target chosen at runtime, and how many of the routes execute?** → `DispatchShape`
- **Is the route set provably exhaustive, or open-world?** → `SetCompleteness`
- **For each target: how do we know it, and under what runtime condition?** → `Route { evidence, condition }`

```rust
pub struct Edge {
    pub from: NodeId,            // app-qualified caller (routine or synthetic root) — §6.2 identity
    pub site: SiteId,            // stable SEMANTIC identity of the originating site — §6.1
    pub kind: EdgeKind,
    pub shape: DispatchShape,
    pub completeness: SetCompleteness,  // EDGE-LEVEL home of the open-world blocker (incl. DynamicOpen
                                        //   via Partial{RuntimeTypeUnbounded}) and of a legal empty fan-out
    pub routes: Vec<Route>,      // 0..N. A genuine failure emits one Unresolved/Unknown route; a legal
                                 //   empty fan-out (no subscribers/implementers) is `[]` + Partial (§3.2)
}

/// WHAT relation the edge expresses. Drives differential domain routing + query stratification.
pub enum EdgeKind {
    Call,             // direct or member procedure call
    Run,              // object-run (Codeunit.Run / Page.RunModal / report/query/xmlport exec) → entry trigger
    ImplicitTrigger,  // data-is-control-flow: Validate→OnValidate, Insert/Modify/Delete→On*, Rename→OnRename
    EventFlow,        // publisher → subscriber(s)
}

/// HOW the target is chosen AND how many routes execute. Honest-dynamic is a SHAPE, not a failure.
pub enum DispatchShape {
    Exact,        // exactly one statically-known route
    Polymorphic,  // ONE route executes at runtime, chosen from the set (XOR): interface/enum/subtype dispatch
    Multicast,    // ALL routes in the set execute (AND), order may be non-deterministic: events, extension triggers
    DynamicOpen,  // target(s) not statically bound at all (e.g. Variant.Run with no type info)
}

/// Is the route set provably complete, or does the open world add more?
pub enum SetCompleteness {
    Complete,   // PROVABLY exhaustive: no installable reverse-dependent could add a route (sealed
                //   interface / explicit closed-world snapshot mode) — NOT merely "we enumerated the
                //   current snapshot". Default to Partial whenever closure is unproven.
    Partial { reason: OpenWorldReason },  // open world may add routes; also the edge-level home of a
                //   DynamicOpen blocker (RuntimeTypeUnbounded) and of a legal empty fan-out.
}
pub enum OpenWorldReason { ReverseDependentImplementers, ReverseDependentSubscribers, ReverseDependentExtensions, RuntimeTypeUnbounded }

pub struct Route {
    pub target: RouteTarget,
    pub evidence: Evidence,
    pub condition: Option<Condition>,   // runtime gate on whether THIS route fires
    pub witness: Witness,               // how this route can be independently checked — §5.5
}

pub enum RouteTarget {
    Routine(NodeId),                               // concrete routine (object-run NORMALIZED to its entry trigger)
    Builtin(BuiltinId),                            // platform intrinsic (catalog hit)
    AbiSymbol { app: AppRef, symbol_key: String }, // KNOWN public boundary whose BODY is unavailable —
                                                   //   retains symbol identity; pairs with Evidence::Opaque
    Unresolved,                                    // genuine failure ONLY — pairs with Evidence::Unknown
}

/// HOW we know a given route. real-unknown counts `Unknown` obligations ONLY (§3.2).
pub enum Evidence {
    Source,   // resolved into ingested source (workspace or ShowMyCode embedded) — strongest
    Abi,      // resolved via dependency .app SymbolReference (public ABI; body available as symbol)
    Catalog,  // platform builtin from the clean-room catalog (carries catalog entry id + version)
    Opaque,   // ABI BODY-UNAVAILABLE boundary ONLY: target is a known public symbol in a stripped dep
              //   whose body we cannot see. NEVER used for a visibility/access conclusion (§5.4).
    Unknown,  // TRUE failure: a site that should have bound did not. The signal to drive to zero.
}

/// Runtime condition under which a route fires (events + conditional triggers). Minimal in 1B.2;
/// the SLOT exists now so event conditionality is not bolted on later.
pub enum Condition {
    RunTriggerGuarded,        // Insert/Modify/Delete(RunTrigger) where RunTrigger is not literal-true
    ManualBinding,            // EventSubscriberInstance = Manual (fires only after BindSubscription)
    SkipOnMissingLicense,
    SkipOnMissingPermission,
}
```

### 3.1 Why `Polymorphic` vs `Multicast` is load-bearing (the analog flaw)

v1's single `Set` lumped interface dispatch with event/extension fan-out. They are semantically
opposite and the difference is the foundation of the charter's "data is control flow" thesis:

- **Polymorphic (XOR)** — interface-typed variable, enum-with-methods, subtype dispatch: exactly
  **one** implementation runs, selected by the runtime type. Downstream effect analysis treats the
  routes as **mutually-exclusive branches**.
- **Multicast (AND)** — event publisher→subscribers, and a base trigger plus all table/page
  **extension** triggers on the same operation: **all** routes run (order non-deterministic).
  Downstream effect analysis must **chain/accumulate** their effects — extension 1's writes are
  visible to extension 2. Collapsing this into a generic set makes reverse-reachability and effect
  accumulation unsound.

### 3.2 The metric: over OBLIGATIONS, not emitted routes

v1's `Unknown routes / total routes` is gameable (zero-route edges vanish from the denominator;
fan-out dilutes a failure; a wrong-but-non-Unknown target reads as success). v2 defines the metric
over **resolution obligations**:

- An **obligation** is a site that *should bind to ≥1 static route*: every `Call`, `Run`,
  `ImplicitTrigger`, and `EventFlow` site is an obligation **unless** it is correctly classified
  `DispatchShape::DynamicOpen` (an honest, proven-dynamic site — its open-world blocker lives at the
  **edge level** in `completeness: Partial{RuntimeTypeUnbounded}`, so it cannot silently vanish).
- Each obligation has exactly **one of four** outcomes:
  - **Resolved** — ≥1 evidenced non-Unknown route.
  - **HonestDynamic** — `DispatchShape::DynamicOpen`; correctly proven dynamic, not a failure.
  - **HonestEmpty** — a **fan-out** site (`EventFlow` / `Polymorphic` / `Multicast`) with **zero**
    routes because the analyzed snapshot genuinely contains none (no subscribers / no implementers /
    no extension triggers), carrying `completeness: Partial{…}`. This is a correct open-world state,
    **not** a failure. (Both reviewers caught v1 mis-counting this as Unknown — every unsubscribed
    BaseApp publisher would have falsely inflated the metric.)
  - **Unknown** — a site that should have bound (`Exact` `Call`/`Run`, or a fan-out the differential
    proves L3 bound) but produced no route. The signal to drive to zero.

```
real_unknown_rate = (# Unknown obligations) / (# obligations)
```
`HonestDynamic` and `HonestEmpty` are honest non-failures, excluded from the numerator. The
guardrail against `HonestEmpty` hiding a real miss: a `Call`/`Run` `Exact` site with no target is
**always** `Unknown` (a direct call must bind); and the differential (§6) flags any site where L3
bound routes but fresh emitted an empty set (`REGRESSION`), so a fresh-side extraction miss cannot
masquerade as `HonestEmpty`.

Reported **stratified** by `EdgeKind` and by **workspace-originated vs dependency-originated**
(charter §8), plus a separate **false-confidence count** surfaced by the gate (§7). The scalar is
never the only gate — the differential (§6) catches wrong-but-non-Unknown targets that the metric
cannot see.

### 3.3 Route identity

Two routes are the same iff `(target, kind, evidence-class, condition, originating-rule)` match —
not target alone. The differential and dedup both use this composite key; the same target reached
by a different semantic rule is a distinct route.

## 4. Architecture

```
AppSetSnapshot (1A)  ──parse_snapshot──►  ParsedUnit[] (bodies: AlFile IR)
        │                                          │
        └──build_program_graph (1B.1)──► ProgramGraph (identity + topology + obj_index)
                                                    │
                              ┌─────────────────────┴───────────────────────┐
                              ▼                                              ▼
                    ResolveIndex (this spec)                       BodyMap (this spec)
        (lookup indexes + WorldMode-aware fan-out)         NodeId → IR routine body + var/param decls
                              │                                              │
                              └──────────────► resolve_program ◄─────────────┘
                                                    │  (this spec)
                                                    ▼
                                              Vec<Edge>  (multi-axis)
                                                    │
                                                    ▼
                                       differential harness (this spec)
                          domain-partitioned diff vs L3 ──► gate (no unverified regression/extra;
                                                            real-unknown ≤ L3; evidence not over-claimed)
```

### Module layout — `src/program/resolve/`

| File | Responsibility |
|------|----------------|
| `mod.rs` | Re-exports; `resolve_program` entry. |
| `edge.rs` | `Edge`, `EdgeKind`, `DispatchShape`, `SetCompleteness`, `Route`, `RouteTarget`, `Evidence`, `Condition`, `Witness`, `SiteId`, obligation accounting + `real_unknown_rate`, `Histogram`. |
| `site.rs` | `SiteId` (semantic, span-based — §6.1); clean-room IR walk extracting syntactic + synthetic sites. |
| `body_map.rs` | `BodyMap`: `NodeId -> &RoutineDecl` (params/locals/body), built during node extraction. |
| `index.rs` | `ResolveIndex` — name-lookup indexes (closure-scoped) + fan-out indexes (`WorldMode`-aware). |
| `receiver.rs` | `ReceiverType` lattice; Phase A `infer_receiver_type`; Phase B `dispatch_member`. |
| `builtins/` | Clean-room catalogs, **generator-driven + versioned** (§5.6): `global.rs`, `member.rs`, `gen/`. |
| `resolver.rs` | Driver: per caller, per site → infer → resolve in the right world → classify → emit evidenced routes. |
| `differential.rs` | Domain-partitioned canonical projection of both engines; semantic-site matcher; diff; adjudication; gate. |

`ProgramGraph` is **not** mutated to carry bodies (decision 1 was "fresh resolver", not
"consolidate"). Bodies are reached through `BodyMap` over already-parsed `AlFile`s.

### 4.1 `ResolveIndex` with explicit `WorldMode`

`resolve_object` (1B.1) covers object-by-name in the caller's closure. v2 adds, each tagged with
the world it is valid in (review found closure-scoping is **wrong** for fan-out):

| Index | Lookup | `WorldMode` |
|-------|--------|-------------|
| routine-overload | `(ObjectNodeId, name_lc) -> Vec<RoutineRef>` (arity disambig) | `CallerClosure` |
| object-by-number | `(from, kind, declared_id) -> Option<ObjectNode>` | `CallerClosure` |
| table-by-id / -name | tables are `Table` objects via the two above | `CallerClosure` |
| table-extension-by-base | `(base_table) -> Vec<ObjectNode>` | **`AnalyzedSnapshot`** + `Partial` if reverse-dependents unanalyzed |
| interface/enum-implementer | `(interface) -> Vec<ObjectNode>` | **`AnalyzedSnapshot`** + `Partial` |
| event-subscriber | `(publisher event) -> Vec<RoutineNode>` | **`AnalyzedSnapshot`** + `Partial` |

```rust
pub enum WorldMode { CallerClosure(AppRef), AnalyzedSnapshot }
```

Name resolution uses `CallerClosure`. Fan-out (Multicast/Polymorphic candidate sets) uses
`AnalyzedSnapshot` and sets `SetCompleteness::Partial` whenever the snapshot cannot prove no
reverse-dependent adds a route. This is the difference between "who can I call" (closure) and
"who reacts to me" (whole analyzed world).

## 5. Resolution semantics

### 5.1 Site extraction (clean-room)

Fresh IR traversal (does not call `ir_walk.rs`; both read the same `al_syntax::ir` types — shared
substrate, not resolver logic). Emits **two site kinds**:

- **Syntactic sites** from `ExprKind::Call`/`StmtKind::Call`: classify callee as `Bare`, `Member`,
  or `ObjectRun` (`Codeunit`/`Page`/`Report`/`Query`/`Xmlport` + run-method). Capture receiver
  `ExprId`, arity, and **source span**.
- **Synthetic sites** with no call expression: data operations on a record receiver
  (`Validate`/`Insert`/`Modify`/`Delete`/`Rename`) → `ImplicitTrigger` sites; the operation's span
  + operation kind + affected table/field is the site's semantic key. Event publisher calls →
  `EventFlow` publisher sites.

### 5.2 Receiver-type lattice (clean-room; named scope so oracle failures aren't surprises)

```rust
pub enum ReceiverType {
    Unknown, SelfObject, ImplicitRec(ObjectNodeId), XRec(ObjectNodeId),
    Record(ObjectNodeId), RecordRef, FieldRef, KeyRef,
    Codeunit(ObjectNodeId), PageOrReport(ObjectNodeId),
    Interface(String), Enum(ObjectNodeId),
    Framework(FrameworkKind),   // HttpClient/JsonObject/TextBuilder/… (catalog-typed)
    Variant,                    // → DynamicOpen
}
```

**Phase A — `infer_receiver_type`** resolves the receiver expression's static type from sources the
review required be named up front: routine **locals**, **parameters**, object **globals**, implicit
**`Rec`/`xRec`**, **`CurrPage`**, the **`with`** stack, page/table/report **trigger context**
(`RoutineDecl.dataitem_source_table` / `enclosing_member` give the implicit `Rec` for member/
dataitem triggers), **return types of calls**, assignment-constrained **`Variant`**,
**`RecordRef.Open`**, **`FieldRef`** provenance, and `Codeunit`/`Report`/`Page` **subtype**
variables. Type names are resolved in the caller's closure. (Phase implementation is staged — §7 —
but the lattice and these sources are specified now.)

**Phase B — `dispatch_member`** maps `(ReceiverType, method)` to routes + shape:

- `SelfObject`/`ImplicitRec`/`Codeunit`/`PageOrReport` → routine-overload lookup → `Exact` (or
  `Polymorphic` over a genuine subtype set). `RouteTarget::Routine`, evidence per target tier.
- `Record`/`RecordRef`/`FieldRef`/`KeyRef`/`Framework` → member-builtin catalog first → `Catalog`;
  else the table's own procedures/triggers.
- `Interface`/`Enum` → implementer index (`AnalyzedSnapshot`) → `Polymorphic`, `SetCompleteness`
  `Partial{ReverseDependentImplementers}` unless closed-world snapshot proves exhaustiveness.
- `Variant`/`Unknown` → `DynamicOpen` + open-world blocker.

### 5.3 Bare / object-run / implicit-trigger / event edges

- **Bare** `Foo(...)`: self object → base object (extension chain) → global-builtin catalog. `Exact`.
- **Object-run**: resolve the run target object, then **normalize the route to its entry trigger**
  (`OnRun`/`OnOpenPage`/…) — `RouteTarget::Routine`, `EdgeKind::Run`. A run by a typed subtype
  variable → `Polymorphic{Complete}` over the subtype's candidates; by an untyped/Variable with no
  subtype → `DynamicOpen`. (No half-lowered `Object` target — review §6.3.)
- **Implicit trigger** (`EdgeKind::ImplicitTrigger`): `Validate(F)`→`OnValidate`(field);
  `Insert/Modify/Delete(RunTrigger)`→`On*`; `Rename`→`OnRename`. Base trigger **plus all
  table-extension** triggers on that table/field → `Multicast`, `AnalyzedSnapshot`,
  `Partial{ReverseDependentExtensions}` if applicable. `Insert(true)` literal → unconditional;
  `Insert(false)` literal → **no edge**; `Insert(var)` → edge with `Condition::RunTriggerGuarded`.
- **Event flow** (`EdgeKind::EventFlow`): publisher → all matching subscribers across the analyzed
  snapshot → `Multicast`, `Partial{ReverseDependentSubscribers}`; each subscriber route carries its
  `Condition` (`ManualBinding`/`SkipOnMissingLicense`/`SkipOnMissingPermission`) where the attribute
  declares it. (Full event modelling deepens in later specs; the slots exist now.)

### 5.4 Evidence assignment (decoupled from visibility — review's black-hole fix)

| Condition on the resolved target | `Evidence` |
|----------------------------------|-----------|
| Target routine's owning node `tier ∈ {Workspace, EmbeddedSource, LocalSourceVerified}` (body parsed) | `Source` |
| Target is a public symbol in a dep with SymbolReference but body available only as ABI | `Abi` |
| Method matched a builtin catalog entry | `Catalog` (+ entry id + catalog version) |
| Target is a **public** symbol in a **stripped** dep whose **body is unavailable** (`RouteTarget::AbiSymbol`, identity retained) | `Opaque` |
| A site that should have bound did not (name in scope, right shape, no target) | `Unknown` |

**Visibility is NOT an evidence input.** A call that the AL compiler would reject is impossible in
source that provably compiled; if our resolver *concludes* an access violation in a source-available
app, that conclusion means **our visibility/identity/preproc model is wrong** → the route is
`Unknown` (or a hard diagnostic), never `Opaque`. `Opaque` is reserved strictly for the ABI
body-unavailable boundary. This prevents over-strict access logic from laundering real resolver bugs
into a clean metric (both reviewers flagged this as the most dangerous single line in v1).

### 5.5 Minimal witness (pulled forward from 1B.3 so evidence can't be over-claimed)

Every non-`Unknown` route carries a `Witness` that makes its evidence **independently checkable
now**, without the full SymbolReference reconciliation (which stays 1B.3):

```rust
pub enum Witness {
    SourceSpan { file: VirtualPath, span: ByteSpan },  // required for every Source route
    AbiSymbol { app: AppRef, symbol_key: String },     // required for every Abi AND Opaque route (boundary symbol)
    CatalogEntry { id: BuiltinId, catalog_version: String }, // required for every Catalog route
    None,                                               // only valid on an Unknown route
}
```

Open-world blockers are **edge-level** (`completeness: Partial{reason}`), not route-local — a
`DynamicOpen` or empty fan-out has no route to carry one. A contract test asserts the witness
variant matches the evidence (no `Source` without a span, no `Abi`/`Opaque` without a boundary
symbol, no `Catalog` without an entry id). This is what lets the gate (§7) reject
**evidence-strengthening**: fresh may not claim `Source` where it cannot produce the span.

### 5.6 Catalog: generator-driven + versioned (not corpus-bounded — review's overfitting fix)

The clean-room catalogs are produced by a **generator** run against a **pinned** AL compiler /
platform artifact (the same authoritative surface the existing `@generated` catalog came from),
emitting entries **versioned by runtime/platform/application**. The existing generated catalog is a
**strict diff oracle**: every old entry must be reproduced or recorded as a deliberate, documented
exclusion. CDO is the **regression corpus that prioritizes work**, never the definition of
completeness — "bounded to what the corpus exercises" is explicitly rejected (it overfits and
regresses on the next app). Catalog surface that is implemented-but-incomplete is declared as such;
the match-or-beat-L3 claim is scoped to the implemented surface and tracked toward full parity.

## 6. The dual-run differential oracle (the spine)

### 6.1 Stable SEMANTIC site identity (ordinals abolished — both reviewers)

v1 aligned the two engines by `(caller, ordinal)`; one missed/added site cascades and poisons the
whole routine. v2 identity is span-based and **matched**, never keyed by position:

```rust
pub struct SiteId {                 // for syntactic Call/Run sites
    pub caller: NodeId,             // ProgramGraph snapshot-qualified node identity (§6.2)
    pub preproc_ctx: PreprocCtxId,  // active #if context (charter C3) — sites differ per compilation context
    pub span: ByteSpan,             // absolute [start,end) of the call expression in its source doc
    pub callee_fingerprint: u64,    // normalized callee token + arity
}
```

Synthetic sites (`ImplicitTrigger`/`EventFlow`) use a **semantic** key instead of a span-callee:
`(caller, edge_kind, operation_kind, affected_table/field/event, rule_version)`.

The aligner is a **matching algorithm**, not key equality: (1) partition by `(caller, edge_kind)`;
(2) exact `span+callee` match; (3) span-overlap + callee-fingerprint; (4) syntax-path; (5) ordinal
only as a final tie-break among genuine duplicates; (6) anything left → **`UNALIGNED`** (a distinct
bucket, **never** counted as a target divergence). One extraction mismatch becomes one `UNALIGNED`,
not a routine-wide cascade.

### 6.2 Canonical node identity = ProgramGraph's, not a weak tuple

The caller/target canonical key is ProgramGraph's snapshot-qualified `NodeId` (app guid+version,
namespace, object kind/id/name, routine name + **parameter signature**, trigger discriminator,
source span) — review found v1's `(appGuid, kind, name, arity)` tuple under-qualified (no
namespace, no version, no signature). L3 emits internal string ids; a thin **L3→NodeId adapter** in
`differential.rs` maps them through app id/version + object kind/id/name/namespace + routine
signature + span. The oracle never uses a weaker identity than the graph itself.

### 6.3 Domain-partitioned diff (L3 is the oracle only where it models the domain)

The diff is partitioned by `EdgeKind`. **L3 is the oracle only for `Call`, `Run`, and the implicit
triggers it already emits.** For edge kinds L3 does not model (new event-flow, new synthetic roots),
the validators are **fixtures + contracts + the §5.5 witnesses**, not L3 — diffing a fresh-only edge
domain against an engine that lacks it would be pure noise.

Within an L3-modelled domain, for each matched site:

A §5.5 witness proves a route's target/evidence **exists**; it does **not** prove the route is
**applicable** from this site (a false fan-out to a real source routine would still be "witnessed").
So a fresh-only target is an automatic `VERIFIED_WIN` **only** where L3 was Unknown at that site;
any fresh-only target at a site L3 already bound differently is a `DIVERGENCE` needing a
fixture-backed applicability proof, never an auto-allowed superset.

| Bucket | Condition | Disposition |
|--------|-----------|-------------|
| **MATCH** | route target-sets equal **and** fresh evidence not stronger than L3's without a §5.5 witness | ok |
| **VERIFIED_WIN** | site where **L3 was Unknown**; fresh binds a target **with** a valid evidence witness | logged, allowed (moat advanced) |
| **UNVERIFIED_EXTRA** | site where **L3 was Unknown**; fresh binds a target **without** a witness, non-dynamic shape | **blocker** (unwitnessed new edge) |
| **REGRESSION** | L3 resolves where fresh is Unknown/empty | **blocker** |
| **EVIDENCE_OVERCLAIM** | fresh claims stronger evidence than it can witness (e.g. `Source` w/o span) | **blocker** |
| **DIVERGENCE** | both bind but target-sets differ — **including a fresh superset** (witness ≠ applicability, see above) | **blocker until adjudicated** with an applicability fixture (§6.4) |
| **MISSING-SITE** | site in L3, absent in fresh | **blocker** (extraction gap) |
| **EXTRA-SITE** | site in fresh, absent in L3 | **blocker until justified** (over-extraction, or a real L3 miss → VERIFIED_WIN) |
| **UNALIGNED** | site could not be matched either way | **blocker until resolved** (alignment bug, not a result delta) |

A false extra edge is as damaging as a missed one (it pollutes impact analysis); v1's blanket
`WIN`-allows-supersets is replaced by the witness-plus-applicability requirement above.

### 6.4 Machine-checkable adjudication (no waiver landfill)

`KNOWN_DIVERGENCES` entries are structured and CI-enforced: `{ reason_code, expected_winner,
witness_or_fixture_ref, owner, date, revisit_condition }`. No glob waivers; every entry must
reference a replaying fixture; the count must monotonically decrease unless a new entry is justified;
CI fails on a stale/unreferenced waiver. A waiver without a fixture is a disabled TODO, not an
adjudication.

### 6.5 Gate (per phase, on CDO + the in-repo fixture corpus)

A phase is "done" only when, over its **in-scope** site predicate (mechanically defined per phase,
§7) on CDO + fixtures:

```
REGRESSION == 0 && MISSING-SITE == 0 && UNALIGNED == 0 &&
UNVERIFIED_EXTRA == 0 && EVIDENCE_OVERCLAIM == 0 &&
unjustified (EXTRA-SITE | DIVERGENCE) == 0 &&
fresh.real_unknown_rate(in-scope) <= l3.real_unknown_rate(in-scope)
```

`VERIFIED_WIN`s are logged, never blocked. Early phases legitimately exclude not-yet-implemented
shapes via the **explicit in-scope predicate** (which also reports the count of excluded
L3-resolved sites, so "not yet implemented" can't hide hard cases); the final phase removes all
filters and runs whole-corpus.

## 7. Internal phases (sequenced; one plan, harness-first)

Each phase ends with an oracle-gated, independently testable deliverable, with a **mechanically
defined in-scope site predicate**.

- **Phase 0 — Edge model + semantic-site identity + harness skeleton.** `edge.rs`; `SiteId` +
  span-based matcher + `UNALIGNED` bucket; L3→NodeId adapter; domain partitioning; obligation
  accounting + stratified `real_unknown_rate`; `--program-call-graph-stats`. Fresh resolver stubs
  one `Unresolved`/`Unknown` obligation per extracted syntactic site. **Gate:** matcher fixture
  matrix green (duplicate/nested/chained/arg/`with`/quoted-ident/preproc-branch/object-run/
  overload/multi-app cases — review's expanded matrix); harness runs on CDO; full gap measured;
  zero `UNALIGNED` on the fixtures.
- **Phase 1 — Site extraction + `ResolveIndex` + `BodyMap`.** Clean-room IR walk (syntactic +
  synthetic sites); the lookup + `WorldMode` fan-out indexes; body map. **Gate (Call/Run domain):**
  MISSING/EXTRA-SITE == 0, UNALIGNED == 0 vs L3.
- **Phase 2 — Core resolution + clean-room global-builtin catalog (generator-driven).** Bare,
  self/extension chain, object-run normalization, global builtins. **Gate (Bare/Run in-scope):**
  REGRESSION/UNVERIFIED_EXTRA/EVIDENCE_OVERCLAIM == 0, in-scope real-unknown ≤ L3.
- **Phase 3 — Receiver lattice + member-builtin catalog (generator-driven).** Phase A/B per §5.2.
  **Gate (Member in-scope):** same predicates on the member subset.
- **Phase 4 — Polymorphic (interface/enum) + Multicast (implicit-trigger/extension/event) edges,
  open-world completeness.** **Gate:** whole-corpus, all filters removed — REGRESSION == 0,
  fresh.real_unknown_rate ≤ l3.real_unknown_rate, all DIVERGENCEs adjudicated, zero UNALIGNED /
  UNVERIFIED_EXTRA / EVIDENCE_OVERCLAIM.

(Plan 1B.3 then adds the full SymbolReference ABI cross-check, the deep re-baseline, and the cutover
that retires the L3 oracle.)

## 8. Testing strategy

- **Unit** per module: edge constructors + obligation accounting + `real_unknown_rate`; the
  witness↔evidence contract; site extraction on the Phase-0 fixture matrix; each `ResolveIndex`
  lookup incl. topology-scope negatives (callee in a non-dependency app invisible) and fan-out
  world-mode (reverse-dependent implementer included only under `AnalyzedSnapshot`, marks `Partial`);
  receiver Phase A inference table (every source in §5.2); Phase B dispatch table; the §5.4 evidence
  truth-table (esp. source-available access-violation → `Unknown`, not `Opaque`).
- **Differential** (the spine): env-gated CDO run (`CDO_WS`) producing domain-partitioned buckets;
  per-phase gate assertions; an in-repo multi-app fixture corpus + frozen mini-oracle snapshot so the
  gate also runs in CI without the CDO workspace.
- **Contracts** (engine-independent, always on): every `Catalog` route's method in the catalog +
  carries entry id/version; no route both `Source` and `Catalog`; `Exact` ⇒ `routes.len() ≤ 1`;
  `Polymorphic`/`Multicast` ⇒ all routes share the dispatch site; witness variant matches evidence
  (`Source`⇒span, `Abi`/`Opaque`⇒`AbiSymbol`, `Catalog`⇒entry id); every `Opaque` route's target is
  `RouteTarget::AbiSymbol` (identity retained, never `Unresolved`); `DynamicOpen` ⇒
  `completeness == Partial{RuntimeTypeUnbounded}`; an empty fan-out (`routes == []`) ⇒
  `completeness == Partial{…}` and counts as `HonestEmpty`, never `Unknown`; an `Exact` `Call`/`Run`
  with no target counts `Unknown`; `real_unknown_rate` counts exactly Unknown obligations.
- **Determinism**: edges sorted by `(from, site)`; routes sorted by composite identity; two runs
  byte-identical (reuses 1B.1's filesystem-independent AppRef ordering).

## 9. Risks & mitigations

| Risk | Mitigation |
|------|------------|
| Site identity misaligns the two engines → phantom MISSING/EXTRA noise | Span-based semantic identity + matching algorithm + `UNALIGNED` bucket; Phase-0 fixture matrix gates before any resolution diffing is trusted. |
| Clean-room catalog less complete than `@generated` → real-unknown regresses on other apps | Generator-driven from the pinned compiler surface, versioned; old catalog a strict diff oracle; CDO is regression-prioritization, not completeness. |
| Receiver-lattice re-derivation drops a hard case L3 handled | The REGRESSION gate is exactly this tripwire; §5.2 names every inference source up front so failures aren't surprises. |
| Over-strict visibility logic launders bugs into a clean metric | Evidence decoupled from visibility (§5.4): source-available access violation → `Unknown`; `Opaque` = ABI-body-unavailable only; evidence truth-table test. |
| Fresh over-approximates (false fan-out / global leak) and it reads as a WIN | Gate splits `VERIFIED_WIN` (witnessed) from `UNVERIFIED_EXTRA` (blocker); `Set` replaced by Polymorphic/Multicast + completeness so fan-out is precise. |
| Fresh claims stronger evidence than it has | §5.5 witnesses required now; `EVIDENCE_OVERCLAIM` is a hard gate bucket; minimal ABI/source witness pulled forward from 1B.3. |
| Fan-out under-approximates by using dependency closure | `WorldMode::AnalyzedSnapshot` for interface/event/extension indexes; `SetCompleteness::Partial` open-world tail when closure unprovable. |
| `KNOWN_DIVERGENCES` becomes a waiver landfill | Machine-checkable entries, fixture-backed, monotonic-decrease, CI fails on stale waivers. |
| Catalog generator is itself a sub-project | It is bounded (mechanical harvest of a pinned artifact + diff against the existing generated set); scoped as its own phase deliverable, not open-ended discovery. |

## 10. Success criteria

1. `resolve_program(&ProgramGraph, &[ParsedUnit])` emits multi-axis `Edge`s over the full CDO
   corpus, panic-free, deterministically, with witnesses on every non-Unknown route.
2. Dual-run gate green at Phase 4: **zero REGRESSION / UNALIGNED / UNVERIFIED_EXTRA /
   EVIDENCE_OVERCLAIM**, all DIVERGENCEs adjudicated with fixtures, and
   `fresh.real_unknown_rate <= l3.real_unknown_rate` (stratified by edge-kind + workspace-origin) on CDO.
3. Every `VERIFIED_WIN` is logged with the construct it newly resolves and its witness — the
   concrete evidence the rebuild advanced the moat without inventing false edges.
4. Contracts hold; output is deterministic; the existing L3 path and its goldens are untouched
   (oracle integrity preserved for 1B.3).
