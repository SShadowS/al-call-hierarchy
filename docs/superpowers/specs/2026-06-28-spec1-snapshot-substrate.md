# Spec 1 — Snapshot Substrate + Evidence + Identity

- **Date:** 2026-06-28
- **Status:** Design (pre-implementation). Serves the
  [charter](2026-06-28-bc-semantic-intelligence-charter.md) §9 step 1.
- **Mandate:** best solution, refactoring on the table (pre-release).

## 1. What this spec delivers (scope)

The **foundation substrate**: turn the engine from *"workspace source + symbol-only dependency
tables"* into *"an explicit app-set snapshot of immutable, identity-verified, per-app source
roots, parsed into one unified IR and resolved through dependency topology."* Concretely:

1. **App-set snapshot model** — an explicit, content-hashed set of apps (workspace + deps +
   optional reverse-dependents), each with a resolved **source provider** and **compilation
   context**, and a closed/open-world flag.
2. **Source providers** — pluggable, identity-verified source acquisition per app: workspace
   files, embedded ShowMyCode `.app` source, verified local repo, symbol-only fallback.
3. **Deep ingestion** — parse *all available source* (workspace + source-shipping deps) into the
   existing owned IR, tagged with per-app provenance.
4. **Canonical node identity** — snapshot-qualified, namespace-aware keys so the same object is
   one node across apps.
5. **The 2-axis edge data structure** — `(DispatchShape, Vec<Route>)` (charter §5), wired into
   the graph so downstream specs populate it.
6. **ABI cross-check verifier** — a shadow pass classifying current edges against SymbolReference
   (proven / refuted / uncheckable) → the soundness baseline.
7. **Deterministic, content-addressed cache** by `.app`/source hash.

**Out of scope (later specs):** the behavior edges themselves (events/triggers/effects — Spec
3/5), per-edge witnesses + CI contracts (Spec 2), the change-impact query (Spec 4), incremental
editor updates (charter E3, later). This spec builds the *substrate they all stand on*.

## 2. Current pipeline (what exists)

- `dependencies.rs` — `find_all_alpackages_folders`, matches `app.json` deps → `.app` files.
- `app_package.rs` — `parse_app_file` reads the `.app` zip, parses **only `SymbolReference.json`**
  → `ExternalObject` (public ABI; the embedded `.al` source is discarded).
- `engine/l3/l3_workspace.rs` — `assemble_and_resolve_workspace_default(workspace)` parses the
  workspace `.al` into IR and resolves L3; `assemble_*` family builds units.
- `engine/deps/cross_app_l3.rs` — merges the workspace L3 with the `.app`-dep symbol objects
  (deps are `external`, public-boundary only).
- `graph.rs` — `CallGraph` (interned, O(1) lookups). `l4/combined_graph` — cross-app combined
  edges.

The gap: deps enter as *symbol tables*, never as *source*. Spec 1 inserts a source-acquisition
+ deep-parse layer **before** resolution, so source-shipping deps become first-class IR units.

## 3. Target architecture (components)

Each component is an isolated unit (clear purpose / interface / deps), built bottom-up.

```
AppSetSnapshot  ── built by ──>  SnapshotBuilder
   │  apps: Vec<AppUnit>                 │ uses
   │  topology: DependencyGraph          ├─ SourceProvider (per app)
   │  world: Closed | Open               ├─ IdentityVerifier
   ▼                                     └─ CompilationContext (per app)
AppUnit
   identity: AppId{ guid, name, publisher, version }
   provenance: { provider, trust_tier, content_hash }
   source: SourceRoot (lazy)            ── parsed by ──>  IR (al-syntax, existing)
   compilation: CompilationContext
   abi: Option<SymbolIndex>             ── from SymbolReference (cross-check)
        ▼
NodeId (canonical, snapshot-qualified, namespace-aware)   ── interned ──>  SemanticGraph
Edge = (DispatchShape, Vec<Route{ target: NodeId, evidence: Evidence }>)
        ▼
AbiCrossCheck  ── classifies edges ──>  { proven | refuted | uncheckable }
```

### 3.1 `SourceProvider` (trait)
Resolves an `AppId` to a `SourceRoot` (set of virtual source files) + a trust tier. Implementors:
- `WorkspaceProvider` — the app under development (truth).
- `EmbeddedAppProvider` — extracts `src/**/*.al` from a ShowMyCode `.app` zip (offset-40 NAVX +
  PK; the `app_package.rs` zip reader already handles the format).
- `LocalRepoProvider` — user-configured `appId/publisher/name/version → local source root`.
- `SymbolOnlyProvider` — no source; yields only the ABI boundary (honest fallback).

Selection priority per app: Workspace > Embedded(verified) > LocalRepo(verified) > SymbolOnly.
A provider returns source ONLY if `IdentityVerifier` passes; else it degrades to `approximate`
or `SymbolOnly`.

### 3.2 `IdentityVerifier`
A source root is **sound** only if it provably matches the artifact under analysis:
- embedded source: implicitly bound to its `.app` (same package) — verify `.app` hash + the
  SymbolReference `AppId`/`Version` match the `app.json` dependency entry.
- local repo: require app id/name/publisher/version match **and** a corroborating signal (git
  commit/tag recorded, or a source hash). Without it → `trust_tier = Approximate` (usable but
  never folded into a "sound" claim; flagged in query evidence).
- mismatch (wrong version / hash) → **fail closed**: drop to `SymbolOnly` + record a provenance
  warning. Never silently analyze mismatched source.

### 3.3 `CompilationContext` (per app)
Each app carries its *own*: preprocessor symbol set, runtime/platform/application versions,
feature flags (from its `app.json` / manifest). Parsing/lowering a dep's `#if` uses **that dep's**
symbols — never the workspace's (phantom-edge prevention, charter C3). For embedded source the
context comes from the `.app` manifest; for the workspace, from its `app.json` + build config.

### 3.4 `NodeId` (canonical identity)
`{ app: AppId, object_kind, object_id, namespace, name }` for objects; `+ member_signature` for
routines. Snapshot-qualified (so Spec 4/D5 can diff across snapshots). Namespace-aware (BC 24+).
Interned. **Resolution uses the dependency topology** (an app sees only its declared dependency
closure), never a flat global name match — this is what makes 11k-file shadowing sound.

### 3.5 `Edge` (the 2-axis structure)
```
struct Edge { kind: EdgeKind, shape: DispatchShape, routes: Vec<Route>, site: Citation }
enum DispatchShape { Exact, Set, DynamicBounded, DynamicOpen }
struct Route { target: NodeId, evidence: Evidence }
enum Evidence { Source, Abi, Catalog, Opaque, Unknown }
```
Spec 1 defines + wires the structure and populates `calls` edges; later specs add edge kinds and
fill richer shapes/evidence. `EdgeKind` starts with `Calls`; the enum is open for
`Publishes/Subscribes/Runs/Triggers/Reads/Writes/...` (Spec 3/5).

### 3.6 `AbiCrossCheck`
Independent verifier: build a canonical `SymbolIndex` from each app's SymbolReference (object id/
type/name/namespace + public member + signature + visibility + compiler-synthesized members). For
every cross-app/public edge route, assert the claimed target exists in the index with matching
signature + visibility. Output per route: `proven | refuted | uncheckable`. **Refuted = a
soundness bug** (false `Source`/`Abi`) → surfaced loudly. This is the first soundness instrument
(Spec 2 turns it into a CI gate).

### 3.7 Cache + determinism
Content-addressed store keyed by `.app`/source-file hash: extracted source, parsed IR facts, the
SymbolIndex. Stable, snapshot-qualified IDs; deterministic ordering everywhere serialized. Warm
rebuild = hash hits. (Incremental *editor* updates are later; the cache + stable IDs are built
now because retrofitting them is expensive.)

## 4. Validation — how we prove Spec 1 works

1. **Deep re-baseline.** On the CDO Cloud workspace (9/10 deps ship source), build the snapshot
   in **deep** mode and re-measure the 2-axis taxonomy. Report stratified: workspace-originated
   vs dep-source vs symbol-boundary (charter §8). The shallow 0.11% is superseded; we expect a
   far larger, deeper graph (dep internals now visible) with a new honest baseline.
2. **ABI cross-check green.** Zero `refuted` routes on the corpus (no false `Source`/`Abi`), or
   every refutation is a real, ticketed resolver bug.
3. **Identity guard works.** Tamper a dep `.app`/version → the verifier fails closed to
   `SymbolOnly` + warning (test).
4. **Determinism.** Two builds of the same snapshot produce byte-identical serialized graph IDs.
5. **No regression.** The existing differential/golden suite stays green (the workspace-only path
   is unchanged when run in shallow mode).
6. **Scale.** Full snapshot (~11k files incl. Base App 8020) builds within a sane budget; warm
   rebuild hits cache. (Perf targets firm up later; this is the smoke test that the architecture
   isn't O(N²).)

## 5. Testing strategy

- **Unit:** each `SourceProvider`; `IdentityVerifier` (match / approximate / fail-closed);
  `CompilationContext` preproc isolation (a dep `#if` resolved with dep symbols, not workspace);
  `NodeId` topology-aware lookup + shadowing; the `Edge` 2-axis invariants.
- **Integration:** snapshot build over a small fixture app-set (2–3 mini apps + a symbol-only
  one); deep vs shallow mode; ABI cross-check classification.
- **Differential:** the real CDO workspace — stratified taxonomy snapshot golden (Rust-owned).
- **Property/robustness:** snapshot build over an ecosystem corpus → zero panics, zero
  Unknown-node lowering (extends the existing `ir_lowering_audit` gate to dep source).

## 6. Non-goals (explicit — deferred)

- Behavior edges (events/triggers/effects) — Spec 3/5.
- Per-edge witnesses + CI soundness contracts + parity-fixture corpus — Spec 2.
- Change-impact / any query — Spec 4.
- Incremental editor-latency updates — later (cache + stable IDs are built now to enable it).
- Opaque dependency summaries — charter non-goal until query-blocking.

## 7. Open questions / risks

- **Memory at scale:** 11k files' IR can't all stay resident. Plan: parse → extract node facts +
  ranges → release AST; re-parse on demand from cache. Confirm the fact-extraction is
  lossless-enough that queries rarely need re-parse. *(Validate during §4.6.)*
- **Embedded-source ≠ compiled reality:** the compiler injects members (`Rec.SystemId`, auto-gen
  extension fields). The `SymbolIndex` must supply these so source-derived nodes reconcile with
  the ABI (else false `refuted`). *(Reconcile in `AbiCrossCheck`.)*
- **`app.json` compilation context fidelity:** can we recover each app's exact preprocessor
  symbols/feature flags from the `.app` manifest? If not fully, mark affected `#if` branches
  `conditional-unverified` rather than guess. *(Spike during implementation.)*
- **Provider config UX:** how the user declares `LocalRepoProvider` mappings (a config file
  keyed by app id). Minimal first: a `.al-call-hierarchy.json` `sourceProviders` block.

## 8. Implementation order (for writing-plans)

1. `AppId` + `AppSetSnapshot` + `DependencyGraph` types; `SnapshotBuilder` skeleton (wraps the
   existing `dependencies.rs` discovery).
2. `SourceProvider` trait + `WorkspaceProvider` + `EmbeddedAppProvider` (reuse `app_package.rs`
   zip reader to pull `src/**/*.al`).
3. `IdentityVerifier` + `SymbolOnlyProvider` + `LocalRepoProvider` + provider selection.
4. `CompilationContext` per app; wire into IR parse/lower.
5. `NodeId` canonical identity + topology-aware resolver lookup (refactor `cross_app_l3`).
6. `Edge` 2-axis structure in `graph.rs`/`combined_graph`; populate `calls`.
7. `SymbolIndex` + `AbiCrossCheck` verifier.
8. Content-addressed cache + determinism pass.
9. Deep re-baseline on CDO + stratified golden; ecosystem robustness gate.
