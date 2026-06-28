# BC Semantic-Intelligence Engine — Goals & Capability Charter

- **Date:** 2026-06-28
- **Status:** North-star charter (living document). Anchors all implementation specs.
- **Mandate:** Best solution, not the simplest/easiest/quickest. Time is not a constraint.
  Pre-release — any refactoring is on the table.
- **Reviewed:** gpt-5.5, gpt-5.5-pro (high), gemini-3.1-pro — twice; the 2-axis edge model
  (§5) and implicit data→behavior edges (§4 B0) come from the second pass.

---

## 1. North-star goal

> **The verified semantic-intelligence layer for Microsoft Dynamics 365 Business Central.**
> For a specific **app-set + version snapshot**, resolve every *statically knowable* **call /
> event / effect** edge across app boundaries — *into* the internals of source-available
> dependencies — with **artifact/source provenance and compiler-grade resolution evidence**.
> Where apps or calls are opaque or dynamic, model them **conservatively and expose the
> uncertainty**. On that substrate, answer the high-stakes questions — change-impact,
> upgrade-risk, event/data/transaction flow, safe refactoring, AI-with-citations — that stop
> dead at the app boundary in every other AL tool.

The graph + per-edge evidence is the **trust substrate**. The **product is the answers.** Users
do not buy "proofs"; they buy *"can I safely change / upgrade / refactor / audit / understand
this BC environment without missing the hidden cross-app behavior?"* The evidence is what makes
the answer trustworthy enough to act on.

## 2. The gap no existing tool fills

Every other AL tool is **app-local** and **boundary-blind**:
- the **AL compiler** resolves against dependency *symbols*, never dependency internals;
- the **AL VS Code extension** (go-to-def / find-references) stops at the app boundary;
- **AppSourceCop / CodeCop** are rule-based and per-app;
- none build a **trustworthy whole-environment semantic model**, and none **prove** their
  resolution is correct.

In BC the interesting behavior lives *across* boundaries — Base App posting pipelines, event
publish/subscribe, table/page/report triggers fired by *data operations*, ISV libraries,
transaction effects, hidden side effects through dependencies. Seeing *through* every
**source-available** app boundary, into dependency source, with evidence, is the differentiator.
**Source-first** is realistic: ~95% of in-scope apps ship full source (ShowMyCode); symbol-only
is the rare, honestly-marked fallback.

## 3. What we build (the substrate)

A **typed, multi-relation, in-memory semantic graph** over an explicit **app-set snapshot** —
not a single call graph.

- **Nodes** (interned; identity = a canonical, **snapshot-qualified** cross-app key, including
  **namespaces**, so "the same object" is one node whether it lives in the workspace or Base
  App): objects (codeunit/table/page/report/**query/xmlport/enum/interface/controladdin/
  permissionset/permissionsetextension/entitlement/profile**, and the **extension** kinds
  table/page/report/enum-extension), routines (procedures/triggers/
  methods), events, tables/fields. Each node carries **provenance** (app, source provider,
  trust tier, content hash), lifecycle (`ObsoleteState`/`Reason`/`Tag`), and a `(file,
  byte-range)` pointer for **citations**.
- **Edges are 1:N polymorphic with per-route evidence** (the core data structure — §5).
  Edge types (the *behavior* graph): `calls`; `publishes`/`subscribes` (synthetic event flow,
  incl. reverse-dependents); `runs` (`Codeunit`/`Report`/`Page.Run`, `TaskScheduler`,
  background); **`triggers`** (implicit data-op dispatch — `Validate`→`OnValidate`,
  `Insert`/`Modify`/`Delete`/`Rename` with `RunTrigger`→table+extension triggers, page/report/
  query/xmlport execution triggers); `reads`/`writes`/`modifies`/`deletes` (data effects);
  `implements` (interface → impl set); `extends` (table/page/report extension → base). Every
  edge carries a **`(DispatchShape, Vec<Route>)`** classification (§5) + a `(file,range)`
  call-site citation.
- **Cross-app & in-memory:** unified via canonical node identity + **dependency-topology-aware**
  resolution (no flat global name match across 11k files); built once per snapshot;
  **content-addressed cached** by `.app`/source hash so warm rebuilds are fast.
- **Node fidelity rule:** promote everything that affects **behavior / data-flow / resolution /
  reachability / lifecycle / security** into the node — types, `temporary`, FieldClass/
  FlowField/CalcFormula/TableRelation, key fields, page SourceTable + actions/triggers, report/
  query dataitems, procedure attributes + by-ref params + scope, extension chains, namespaces,
  ObsoleteState, DataClassification, AccessByPermission, object `Permissions`. Pure presentation
  (captions, tooltips, layout) stays in source — **"skip" never means "lose,"** it means kept in
  source and fetched on demand via the range pointer.

Existing fragments to unify/deepen (not rebuild): `graph.rs` (CallGraph), `l4/combined_graph`
(cross-app), `l3/event_graph`, the L5 effect detectors, `app_package`/`symbol_reference` (`.app`
ingestion — currently symbols only, discards embedded source).

**Queries are traversals over this graph.** Every answer reports the trust tiers and
opaque/dynamic blockers it crossed.

---

## 4. Capability subgoals — what the engine MUST be able to do

### A. Whole-program resolution (the substrate)
- **A1. App-set snapshot, not dependency-closure.** Ingest an explicit analyzed app-set /
  environment snapshot: workspace app(s), dependencies, and (when relevant) installed /
  **reverse-dependent** apps. Apps outside the snapshot are never silently assumed absent;
  completeness-sensitive queries report **open-world** blockers. Closed-world vs open-world is
  an explicit query mode.
- **A2. Source-first ingestion.** Embedded ShowMyCode source as the primary input; a
  **local-source provider** (map `appId/publisher/name/version → local repo root`) for apps
  whose `.app` hides source but the user owns it; symbol-only as honest fallback.
- **A3.** Parse all available source into the unified owned IR across all apps.
- **A4. Resolve into source-available internals.** Resolve every statically-knowable call edge
  *into* the internals of source-available dependencies (locals/triggers/bodies). For
  symbol-only / unavailable apps, resolve to the **ABI boundary** and classify accordingly
  (§5) — never guess across an opaque boundary.
- **A5.** Build one in-memory typed semantic graph spanning all apps with canonical, snapshot-
  qualified cross-app node identity (namespace-aware).
- **A6.** Merge **object extensions** (table/page/report-extension) into the effective surface
  of their base objects (members, triggers, fields).
- **A7. Object-kind coverage.** Model every behavior-relevant AL object kind: codeunit, table,
  page, report, query, xmlport, enum, interface, controladdin, permissionset(+extension),
  entitlement, profile, and the extension kinds (table/page/report/enum-extension) — the latter
  as first-class nodes with their own identity/provenance, *and* merged into their base (A6).

### B. Behavior graph (data is control flow)
- **B0. Implicit data→behavior edges (first-class).** Treat data operations as dispatch:
  `Rec.Validate(Field)`/`FieldRef.Validate` → field `OnValidate` (table + all extensions);
  `Insert`/`Modify`/`Delete` with `RunTrigger=true` → `OnInsert`/`OnModify`/`OnDelete`
  (table+extensions); `Rename` → `OnRename` (its own trigger model, not a `RunTrigger` flag);
  page/report/query/xmlport execution → their lifecycle triggers. Without these, reverse-
  reachability (D1) silently breaks.
- **B1. Event flow with conditionality.** Resolve `Integration`/`Business`/`Internal`-Event
  publishers to `[EventSubscriber]` subscribers (synthetic edges), incl. **reverse-dependents**.
  Model conditionality: `EventSubscriberInstance=Manual` (requires/bounded by `BindSubscription`),
  `SkipOnMissingLicense`/`SkipOnMissingPermission`, `IncludeSender`, manual `Bind`/`Unbind`.
  Subscriber execution order is **non-deterministic** — never assume an order.
- **B2. Indirect dispatch.** `Codeunit/Report/Page.Run`, `TaskScheduler`/background sessions;
  **interface dispatch** as a *set* of possible implementations (§5 `Set`). Note: interface
  impls are often selected via **enum extensions**, and any reverse-dependent app may add one —
  so interface dispatch is inherently `DynamicOpen` unless the query enforces closed-world
  snapshot mode (A1).
- **B3. Data effects (incl. dynamic).** Typed-record `read`/`write`/`modify`/`delete`, plus
  **`RecordRef`/`FieldRef`/`Variant`** dynamic table/field access (classified exact /
  bounded-set / dynamic). FlowField/CalcFormula/TableRelation data dependencies as edges.
  `TransferFields` is an implicit schema-driven N:N batch write — expand to discrete field-level
  `write` edges by shared column IDs in the snapshot, or data-flow queries get dead zones.
- **B4. Transaction & state effects** (the L5 detectors, ecosystem-wide): `Commit`,
  `TryFunction`, temporary-record state, SingleInstance, error behavior. Note: **`Isolated`
  event** publishers do not roll back / halt sibling subscribers on a crash — error-propagation
  edges must not traverse backward through an `Isolated` boundary (false "unsafe transaction").
- **B5. Session/company context.** Model `ChangeCompany`, cross-company/session/`TaskScheduler`
  boundaries so data-flow does not falsely cross company/session scopes.
- **B6. Entrypoints / roots.** Page actions + triggers, report/query/xmlport execution,
  web-service/OData/SOAP/API exposure, install/upgrade codeunits, job queue, event subscribers,
  test codeunits, external (.NET / control-add-in) boundaries.
- **B7. Security / exposure surface.** Model `permissionset`(+extension), `entitlement`, object
  `Permissions`, `AccessByPermission`, API/web-service exposure, `DataClassification` — as node
  metadata + edges for security/governance queries (full authorization proof is later; the
  schema must not ignore these now).

### C. Soundness & provenance (the trust)
- **C1. Per-edge evidence/witness:** the candidate set considered, the rule that selected each
  route, and (for unknown/dynamic) the lookup trace explaining *why* nothing bound.
- **C2. Source-identity verification:** source is "sound" only if it provably matches the
  artifact under analysis (app id/version + package hash + source hash; for local repos, git
  commit/tag + reproducible-build evidence). Unverified local source → **"approximate,"** never
  folded into a sound claim. Embedded ShowMyCode source outranks a random local checkout.
- **C3. Per-app compilation context:** each app's own preprocessor symbol set, runtime/platform/
  application versions, feature flags — evaluated **per app**, so dependency `#if` branches are
  never resolved with the workspace's symbols (phantom-edge prevention).
- **C4. Cross-app visibility & lookup precedence:** `internal`/`protected`/`local` are *visible
  to the analyzer* but the compiler rejects illegal calls — the engine enforces AL access +
  lookup precedence + dependency-topology shadowing. Source availability never relaxes access.
- **C5. ABI cross-check** against SymbolReference: the compiler's ABI output is the independent
  oracle for object identity, public signatures, and **compiler-synthesized members**
  (`Rec.SystemId`, auto-generated extension fields). Source parsed by our own resolver is
  *input*, not self-proof; the ABI catches resolver drift.
- **C6. Compiler-parity fixture suite:** a curated corpus encoding AL lookup-precedence /
  shadowing / visibility / overload semantics — the independent oracle for cases SymbolReference
  cannot express.
- **C7. No false certainty (existential principle).** Every route, edge, and query answer states
  what is exact / conservative / dynamic-bounded / opaque / unknown. Never claim completeness we
  cannot back. For order-dependent effect claims, model **all subscriber permutations** or
  decline the certainty. One confidently-wrong answer destroys trust; honest gaps are tolerated.
- **C8. Determinism & reproducibility:** stable snapshot-qualified node/edge IDs, stable
  ordering, no hash-map iteration nondeterminism in serialized output, OS/path-separator
  independence. Witness hash-keys (content + app.json + dep hashes + compiler/runtime + preproc
  symbols + catalog + engine version) designed in from Spec 1 — incremental correctness is a
  soundness concern, not a perf afterthought.

### D. Query layer (the answers the substrate must support)
- **D1. Change-impact (the wedge):** for a selected symbol/change, reverse-reachability across
  the snapshot (`calls` + `subscribes` + `runs` + **`triggers`**) → every impacted path with
  source citations, dependency-internal hops, and explicit opaque/dynamic blockers.
- **D2. Event-flow tracing:** forward `publishes`→`subscribes` across apps (order-agnostic).
- **D3. Effect queries:** "can this path `Commit`?", "what `writes` this field?", "can <data>
  reach this sink?" — over the behavior graph, honest about subscriber-order non-determinism and
  company/session scope.
- **D4. Reachability / dead-code:** requires **negative proof** + complete root modeling (B6);
  answered only when coverage supports it, else **blocked-by-opaque/dynamic** with reasons.
- **D5. Upgrade-diff:** version-aware comparison across snapshots ("what changes vN→vN+1 affect
  my customizations + ISV integration"), using snapshot-qualified IDs + a **semantic-counterpart
  mapping** across versions; surfaces obsolete/breaking transitions.
- **D6. Query-level evidence:** every answer reports which apps/versions were included, which
  source was exact / approximate / symbol-only, which regions were opaque/dynamic, which roots
  were modeled, and whether the answer is **complete / conservative / blocked**.

### E. Performance & scale (so it is usable, not a vanity batch job)
- **E1.** Handle the full snapshot in memory (11k+ files; Base App alone ~8k) without blowup
  (interned/arena model; source released after fact-extraction; re-parse on demand from cache).
- **E2.** Content-addressed caching keyed by `.app`/source hash — fast warm rebuilds. *(Cache +
  stable identity are substrate-era decisions, not deferred.)*
- **E3.** Incremental updates for the active workspace — editor-grade latency, not minutes.
- **E4.** Indexed, dependency-topology-aware lookup — no O(N²) global name scans across 11k
  files.

---

## 5. The honest taxonomy — a 2-axis edge classification

**Every edge is `(DispatchShape, Vec<Route>)`**, NOT a flat enum. (An event publisher can reach
a local subscriber, a Base App subscriber, and a symbol-stripped ISV subscriber at once — a flat
category cannot express that; each *route* carries its own evidence.)

**Axis 1 — DispatchShape** (how many targets):
- `Exact` — exactly one statically-determined target.
- `Set` — a bounded set of targets (interface dispatch, event fan-out, extension trigger fan-out).
- `DynamicBounded` — runtime-typed but provably bounded to a candidate set (e.g. a `Variant`
  constrained by assignments).
- `DynamicOpen` — unbounded / open-world (e.g. open interface impls outside the snapshot,
  unconstrained `RecordRef`). *Honest, not a failure.*

**Axis 2 — Evidence, PER route** (how each target is proven):
- `Source` — resolved into available source, visibility + signature satisfied. *(primary, ~95%)*
- `ABI` — cross-app boundary verified against SymbolReference (object + public member +
  signature + visibility).
- `Catalog` — platform intrinsic, verified against the **versioned** builtin catalog.
- `Opaque` — target in a symbol-only / unavailable app: **ABI/Catalog-resolved to the public
  boundary**, but its *internals* are unavailable. (Not "unproven" — `Opaque` describes the
  internal-availability limit, orthogonal to the boundary being ABI-proven.) *(honest)*
- `Unknown` — a route we **expected to resolve and could not**. **The only signal to eliminate.**

**Real-`unknown` = routes with `Evidence::Unknown`** (a binding we should have made but didn't).
`DynamicOpen`/`Opaque` with honest bounds are tracked separately and are **not** failures. Plus
existing diagnostic sub-states: `ambiguous`, `memberNotFound`.

## 6. Principles (non-negotiable)

1. **No false certainty** — honesty over flashiness; conservative under uncertainty; never claim
   completeness we cannot prove; model order-dependence or decline the claim.
2. **Soundness of positive claims** — a stated route/edge/answer is correct, or it is explicitly
   marked conservative/uncertain.
3. **Data is control flow** — data operations (`Validate`/`Insert`/…) are dispatch edges, not
   inert effects.
4. **Source-first, opaque-honest** — optimize hard for the source-available 95%; mark the
   symbol-only minority honestly; never guess across an opaque boundary.
5. **Evidence for everything** — every route and answer carries provenance/citations.
6. **Independent verification** — our resolver's output is cross-checked by the compiler ABI + a
   parity fixture corpus; we do not prove ourselves with ourselves.
7. **Best solution; refactoring on the table** — pre-release; correctness and architecture beat
   expedience.

## 7. Non-goals / deferred (explicit YAGNI — revisit when a real need appears)

- **Opaque dependency *summaries*** (an ISV ships per-API effect/call summaries without source)
  — deferred until opaque apps become **query-blocking** in target corpora.
- **Full product UI** (VS Code impact panels, etc.) — foundation built *product-aware* so
  surfaces sit on it cleanly, but UI is not built first.
- **Automated refactoring *execution*** — needs negative-proof + breaking-change/AppSourceCop
  rules + downstream-impact; the graph powers it later, it is not the first flagship.
- **AI chat as the core** — AI is an *interface* on the substrate (answers with citations), not
  the moat.
- **Full authorization/security proof** — schema models permissions/exposure now (B7); proof
  later.
- **SaaS / multi-tenant hosting & source-IP/legal model** — local/on-prem first.

## 8. Definition of done / how we measure

- **Primary, CI-gated metric:** **workspace-originated real-`unknown` rate → 0** (routes with
  `Evidence::Unknown` originating in workspace edges), with **zero false-`Source`/`ABI`**
  (soundness). Workspace-originated denominator so a regression is not drowned by Base App's ~8k
  uniform files.
- **Stratified reporting** (never one aggregate): workspace / deep-source / symbol-boundary /
  synthetic(event,trigger) / dynamic — separately.
- **Risk-weighted, not raw:** rank residual unknowns by **centrality / query-blocking impact**
  (an unresolved dispatch in a posting pipeline ≫ 1,000 trivial resolved calls). Track a
  **false-confidence rate** (claimed-exact that the ABI/parity oracle refutes) as a first-class
  quality number.
- **Robustness:** zero panics and zero `Unknown`-node lowering across an ecosystem corpus of
  100s of real apps; ABI cross-check green everywhere; deterministic output.

## 9. Roadmap (the specs that serve this charter)

Sequenced so evidence + soundness exist from day one and the wedge proves the foundation early
(per second-review guidance: don't defer contracts, don't wait for the *full* behavior graph
before the wedge):

1. **Spec 1 — Snapshot Substrate + Evidence + Identity.** App-set snapshot model (closed/open-
   world); source-provider with **identity verification**; per-app compilation context; unified
   namespace-aware cross-app node identity + dependency-topology resolution; cross-app visibility;
   ABI cross-check; node-fidelity model; the **2-axis `(DispatchShape, Vec<Route>)` edge
   structure** (§5); content-addressed caching + stable IDs. *Evidence shape designed in here.*
2. **Spec 2 — Resolver Soundness Contracts.** ABI cross-check gate, parity-fixture skeleton,
   determinism, fail-closed `Unknown`, false-confidence metric — running from the first resolver
   change, not deferred.
3. **Spec 3 — Minimum behavior for the wedge.** `calls` + `publishes`/`subscribes` (+ Manual-bind
   conditionality) + `runs` + **`triggers`** (implicit data→behavior). Enough behavior for D1.
4. **Spec 4 — Change-impact wedge** (D1) + query-level evidence (D6): reverse-reachability with
   citations, dependency-internal hops, opaque/dynamic blockers.
5. **Spec 5 — Full behavior graph** (B3 dynamic data effects, B4 transaction/state, B5 session/
   company, B7 security/exposure, RecordRef/FieldRef).
6. **Continuous tracks:** residual→0 burn-down ∥ ecosystem robustness ∥ performance/incremental
   (E) ∥ event-flow/effect/upgrade-diff queries (D2–D5) ∥ completeness long-tail.

> Sequencing rationale: the substrate (1) carries the evidence/identity model; soundness
> contracts (2) run from the first resolver change; the wedge (3–4) proves the graph matters and
> forces the foundation to be right; the full behavior graph (5) and everything else are
> prioritizable only once we can *measure and prove*.
