# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
- **(resolve) Source shadows builtin — lookup-precedence soundness fix +
  structural builtin-catalog match (beyond-1B.3b Task 1)**
  (`src/program/resolve/resolver.rs`, `src/program/resolve/builtins.rs`,
  `src/program/resolve/member_catalog.rs`, `tests/r0-corpus/ws-builtin-shadow/`
  NEW, `tests/program_resolve_harness.rs`) — `resolve_member`'s `Record`
  receiver arm was **catalog-FIRST**: a user/source table procedure whose
  name+arity coincided with a genuine platform-intrinsic Record method (e.g.
  `FieldNo`, `SetRecFilter`) was mis-classified `Evidence::Catalog` instead of
  the correct `Evidence::Source` — AL semantics say a visible source/ABI
  routine SHADOWS a same-named intrinsic. This was the root cause behind the
  42 `builtin-catalog-fp-collision` semantic-audit divergences. Fixed by
  gathering every visible source/ABI candidate across the base table AND its
  TableExtensions FIRST, with explicit cardinality semantics: exactly one
  candidate → `Source`/`Abi`/`Opaque`; **more than one → honest ambiguous
  `Unknown`** (source ambiguity still shadows the catalog — never pick-first,
  never fall through to a false intrinsic); zero candidates (or an
  unresolved table) → consult the Record builtin catalog, preserving the
  existing table-independent-builtin behavior. `resolve_bare`'s own-object
  precedence was already source-before-catalog (investigated and confirmed
  correct pre-fix; kept as a regression-locking fixture, not a second bug).
  Added `global_builtin_id_checked`/`member_builtin_id_checked` — fail-closed
  structural guards re-verifying the catalog hit's canonical NAME (and
  implicit receiver-kind scoping, already enforced by per-kind `phf::Set`s)
  before returning a `Catalog` route; all `resolve_bare`/`resolve_member`
  catalog consult sites now go through the checked wrapper. **Investigation
  note:** the catalog membership check is an exact-lowercase-string `phf::Set`
  lookup (no fingerprint/hash digest is stored or compared anywhere in this
  path — confirmed by reading `builtins.rs`, `member_catalog.rs`, and
  `abi_ingest.rs`'s `param_type_fp`/`fnv1a`, which fingerprints ABI routine
  *signatures* for `RoutineNodeId` identity, an unrelated concern), so a true
  hash collision cannot occur today; the checked wrappers make that invariant
  an executable contract (defense-in-depth) rather than an implicit property.
  **Qualified-intrinsic bypass investigation:** the IR CAN represent a
  fully-qualified platform call (`System.CreateGuid()` parses as an ordinary
  `Member { receiver: "System", method: "CreateGuid" }`); no special-case code
  was needed for the bypass because `Framework`-singleton receivers
  (`System`/`Session`/`NavApp`/...) are classified unconditionally in
  `infer_receiver_type`'s Step 1 (before any variable/source lookup) and
  `resolve_member`'s `Framework` arm is catalog-or-`Unknown` only — it never
  consults source candidates, so a local procedure structurally cannot shadow
  a qualified platform call. New `tests/r0-corpus/ws-builtin-shadow/` fixture
  (5 scenarios, asserted via 5 new `tests/program_resolve_harness.rs` Test-21
  cases with exact route/evidence/target assertions) + 2 new
  `resolver.rs` unit tests (genuine shadow + cross-TableExtension ambiguity) +
  2 new catalog-layer unit tests (near-miss-name fail-closed regression).
  Verified: all pre-existing `resolve_member`/`resolve_bare` tests (50) still
  green; `cargo test --workspace` (no `CDO_WS`) fully green; `cargo clippy
  --release --all-features -- -D warnings` clean; `cargo fmt --check` clean.
  No `engine::l3`/`engine::l2` import added.

### Added
- **Plan 1B.3b Task 4 (CAPSTONE): the fresh engine stands alone — L3 oracle
  retired from validation, verified + honestly documented**
  (`CHANGELOG.md`; no source changes — verification + docs only) — closes
  1B.3b and the whole 1B.3 resolution arc. 1B.3b retires the L3 oracle from
  the fresh resolver's **validation**. As of this task the engine is
  validated by three things, NONE of which call L3 at run time:
  (a) **committed, anonymized, frozen L3-verdict goldens** — Member/Interface
  (`cdo-anon.json`), ImplicitTrigger (`cdo-trigger-anon.json`), EventFlow
  (`cdo-event-anon.json`) — keyed by per-site target identity, which is the
  source of COMPLETENESS evidence; the CDO-scale floor is active on the
  gated/internal runner that has the CDO workspace, public CI validates the
  goldens' metadata (schema version, non-empty, `genuine_wrong==42` against
  the committed manifest) without needing the workspace; (b) the
  **L3-independent contracts** — `coverage_holds`, `evidence_overclaim`,
  `abi_unmapped` (`abi_ingestion_integrity`), and `route_applicability`
  (carrying the Task-2-ported fan-out applicability teeth) — these are
  SOUNDNESS checks: every emitted route is individually well-formed and
  applicable, re-derived independently of any L3 projection, plus the
  Histogram + real-unknown-rate ceiling; (c) **always-run synthetic semantic
  fixtures** (`tests/fixtures/semantic-golden/`, `implicit-trigger/`,
  `fanout-applicability/`, the EventFlow two-stage-join fixture) that need no
  `CDO_WS` at all. Stated plainly, per the plan's honesty framing: this is
  **not first-principles semantic correctness** — it is the FROZEN
  HISTORICAL L3 verdict (captured before retirement) plus the L3-independent
  contracts plus fixtures. The teeth prove SOUNDNESS; the frozen goldens
  carry COMPLETENESS; neither alone would be enough. L3-minting moved
  entirely to the dev-only `mint-goldens` tool (`src/bin/mint-goldens.rs` +
  `src/program/l3_mint.rs`, gated behind `CDO_WS`+`CDO_ANON_KEY` or
  `REGEN_TEMP_GOLDENS=1`); `src/engine/l3` itself STAYS in the tree
  unchanged — it remains the `aldump`/L4/L5 backbone, a separate consumer
  from the fresh resolver; `builtins.rs::global_builtins` (clean-room global
  builtin catalog membership, sourced from `engine::l3::global_builtins`
  data, not logic) remains the one sanctioned `engine::l3` data dependency
  inside `src/program/resolve/`. The fixed, committed anonymization salt
  (`CDO_ANON_KEY` fallback test key) keeps the frozen goldens byte-reproducible;
  `ENFORCE_CDO_WS=1` hard-fails (rather than silently skipping) a
  gated/internal run that loses its `CDO_WS` or hits a zero-site audit; a
  workspace-SHA drift warning (when the live `CDO_WS` content no longer
  matches the SHA the goldens were minted from) is informational only —
  the audits load the frozen goldens regardless, so drift does not fail the
  build.
  **Capstone verification performed for this task** (binding requirement,
  not just narrative): `cargo test --workspace` with no `CDO_WS` set —
  **1610 tests passed, 0 failed**, across 159 test-result blocks (lib +
  every integration test binary + doctests), fully green without the
  oracle; `cargo clippy --release --all-features -- -D warnings` — clean,
  zero warnings; `cargo fmt --check` — clean, no file needs reformatting;
  `grep -rnE "use .*engine::l3|use .*engine::l2" src/program/resolve/` —
  the only hits are in `builtins.rs` (two `use` statements plus one doc
  comment naming the same exception), confirming zero other `engine::l3`/
  `engine::l2` imports anywhere under `src/program/resolve/`. The five
  frozen CDO audits/teeth were each run SINGLY (not as the full suite, which
  cannot run in parallel — unrelated pre-existing constraint) against the
  real, currently-dirty CDO workspace with `CDO_WS` +
  `ENFORCE_CDO_WS=1`, all green and deterministic: `cdo_l3_semantic_audit_no_fresh_wrong`
  (`genuine_wrong=42` exact manifest match, `paired=11377` checked sites,
  `fresh_wrong=174`→`fresh_ahead_dispatch=132`+`genuine_wrong=42`);
  `cdo_trigger_audit_frozen_load` (`matches=185`, `fresh_wrong=0`);
  `cdo_event_audit_frozen_load` (`matched_pairs=2`, `pair_l3_only=0`);
  `route_applicability_zero_violations` (`total_routes=17241`,
  `violations=0`, `abi_unmapped=0`); `fan_out_applicability_zero_violations`
  (all four fan-out violation counters `0`, non-vacuous
  `routes_checked[interface=28 instance_builtin=449 implicit_trigger=958
  event=2284]`). No workspace-SHA drift warning printed on this run.
  **Out of scope for 1B.3b** (explicitly deferred, tracked in the roadmap):
  `genuine_wrong=42` underlying disambiguation (mostly L3-error-on-builtins);
  full `fresh⊆l3` partial-recall validation; the same-arity-type overload
  DISPATCH (Cat-D, 17 divergences); the snapshot double-include root cause;
  table/page/database trigger-events as EventFlow; `BindSubscription`
  activation; the receiver-gap buckets; a workspace-pinning operational doc.
  **The fresh engine now stands alone**: it validates itself, at run time,
  without ever calling into `project_l3*` — L3 is reachable only from
  `src/engine/l3` (the unrelated `aldump` backbone) and from the opt-in
  dev-mint path.

- **Plan 1B.3b Task 2: port fan-out applicability teeth (soundness) into `route_applicability`**
  (`src/program/resolve/semantic_golden.rs`, `tests/program_resolve_harness.rs`,
  `tests/fixtures/fanout-applicability/` NEW; commits `dfec53e` + `1ee0e8e`) —
  ports the four fan-out applicability predicates that previously lived ONLY
  inside the (Task-3-deleted) dual-run gates' FreshOnly branches into
  `route_applicability`, now running over EVERY fan-out route in
  `resolve_full_program`'s full edge set instead of only the FreshOnly-vs-L3
  subset: Interface (`DispatchShape::Polymorphic`) via
  `interface_route_applicable`; instance-builtin/enum-static Catalog `Builtin`
  routes (`PageInstance::`/`ReportInstance::` via
  `instance_builtin_route_applicable`, `Enum::` via the `Enum` member-builtin
  catalog directly); ImplicitTrigger (`DispatchShape::Multicast`) via
  `implicit_trigger_route_applicable` (`Validate` sites fall back to the
  documented table/extension-identity check); EventFlow via the already-`pub`,
  L3-free `differential::verify_event_subscriber_route`. New private
  `build_fan_out_site_context` re-walks the same parsed call sites
  `resolve_full_program` resolves to recover the Interface/`RecordOp`
  call-site context (`FanOutSiteContext`) `Edge`/`Route` cannot carry —
  keyed by `SiteId` so it lines up 1:1 with the edges (incl. all five DML ops
  — Insert/Modify/Delete/Rename/Validate — via `record_op_kind_for_method`);
  fails CLOSED (counts a violation) when no context is recovered for a
  Polymorphic/Multicast edge. `ApplicabilityReport` gains four SOUNDNESS
  counters (`interface_applicability_violations`/`instance_builtin_violations`/
  `implicit_trigger_violations`/`event_violations`, summed by
  `fan_out_violations()`) plus four `*_routes_checked` non-vacuity denominators
  — documented as SOUNDNESS (every emitted route is individually
  well-formed/applicable), distinct from the frozen L3-validated goldens'
  COMPLETENESS. `is_clean()` now requires all six violation counters to be
  zero. 12 new unit tests prove each predicate's positive AND
  fabricated-negative case bites (hand-built `Edge`/`Route`/`FanOutSiteContext`
  fixtures) plus the fail-closed-on-missing-context cases. New on-disk fixture
  `tests/fixtures/fanout-applicability/` exercises all four dispatch kinds
  end-to-end through `resolve_full_program` (Test 20,
  `fan_out_applicability_zero_violations`): `violations==0` on the fixture AND
  (env-gated) on the real CDO workspace — `total_routes=17241`, `violations=0`,
  `routes_checked interface=28/instance_builtin=449/implicit_trigger=958/event=2284`
  (non-vacuous), deterministic. `differential.rs`/`applicability.rs` untouched
  (every predicate needed was already `pub`); `project_l3*` and the dual-run
  gates stay intact for Task 3.

- **Plan 1B.3b Task 1: committed anonymized frozen goldens (all dispatch kinds) + dev-mint tool + `ENFORCE_CDO_WS` guard**
  (`src/program/resolve/anon.rs` NEW, `src/bin/mint-goldens.rs` NEW,
  `src/program/resolve/semantic_golden.rs`, `src/program/resolve/differential.rs`,
  `src/program/resolve/mod.rs`, `tests/program_resolve_harness.rs`,
  `tests/fixtures/implicit-trigger/` NEW, `tests/goldens/semantic-edges/cdo-anon.json`,
  `cdo-trigger-anon.json`, `cdo-event-anon.json`, `implicit-trigger-fixture.json`,
  `.gitignore`, `Cargo.toml`) — the C1 FREEZE that precedes 1B.3b's L3-oracle
  removal (Task 3): every L3-derived correctness baseline the gate module
  depends on is now a COMMITTED, ANONYMIZED, frozen artifact instead of a
  live L3 mint on every run. `anon::anon(domain, s)` is a domain-separated,
  versioned, HMAC-SHA256 keyed hash (`site:v1`/`target:v1`/`trigger-op:v1`/
  `event-pair:v1`); the key comes from the non-committed `CDO_ANON_KEY` env
  var (a committed fallback test key keeps `cargo test --workspace` and the
  synthetic fixtures deterministic without ever anonymizing real CDO data —
  see `anon.rs`'s module docs for the full governance writeup). The dev-mint
  tool (`cargo run --release --bin mint-goldens`, `CDO_WS`+`CDO_ANON_KEY` set)
  is the LAST sanctioned L3 use: it mints + anonymizes the three committed
  goldens (`cdo-anon.json` Member/Interface via `mint_l3_validated_golden`,
  `cdo-trigger-anon.json` ImplicitTrigger via the newly-`pub`
  `project_l3_implicit_trigger_in_scope`, `cdo-event-anon.json` EventFlow via
  the new `CanonicalKey`-keyed `project_l3_event_rows` — sidesteps L3's
  proprietary `stable_routine_id` scheme so the fresh side can independently
  re-derive the same identity) and the gitignored local de-anon map
  (`cdo-deanon-map.json`, `AnonId -> plaintext`, for root-causing a failing
  anonymized diff). `run_cdo_semantic_audit` now LOADS the committed golden
  and anonymizes the fresh side at audit time instead of calling `project_l3`
  live — zero `engine::l3` imports in any `run_cdo_*_audit` function. Two new
  audits (`run_cdo_trigger_audit`/`run_cdo_event_audit`) prove the same
  mechanism for ImplicitTrigger/EventFlow (mechanism-proof scope only — the
  zero-tolerance gates for those dispatch kinds remain the live, CDO-gated
  `run_implicit_trigger_harness`/`run_event_flow_gate`, unchanged, until
  Task 3). The `ENFORCE_CDO_WS=1` hard-fail guard (`cdo_ws_or_enforce`/
  `enforce_audit_ran` in the test harness) makes a missing `CDO_WS`, a
  missing/invalid frozen golden, or a zero-site audit PANIC on the
  gated/internal runner instead of silently skipping — no fail-open. A new
  unconditional, no-`CDO_WS`-needed test validates the three committed
  goldens' metadata (schema version, non-empty, `genuine_wrong==42` via the
  pre-existing `known-genuine-divergences.json` manifest) for public CI. The
  always-run `event_fixture_two_stage_join` fixture test and a new
  `implicit_trigger_fixture_resolves_exact_target_set` fixture test both
  moved off live L3 entirely (`project_fresh_event_rows`/
  `mint_fresh_golden_for_kind` are pure fresh-side, no `engine::l3` build) —
  the always-run, L3-INDEPENDENT semantic coverage these two dispatch kinds
  keep after L3 retirement. Verified frozen==live against the real CDO
  workspace: `genuine_wrong=42` (exact manifest match), EventFlow
  `matched_pairs=2`/`pair_l3_only=0` (matches the documented thin-oracle
  baseline), both audits deterministic across reruns.

- **Plan 1B.3a Task 4 (CAPSTONE): L3-validated semantic edge golden + CDO audit + route-applicability contract**
  (`src/program/resolve/semantic_golden.rs` NEW, `src/program/resolve/mod.rs`,
  `tests/program_resolve_harness.rs`, `tests/fixtures/semantic-golden/`,
  `tests/goldens/semantic-edges/fixture.json`) —
  captures the post-L3 correctness floor before L3 retirement in 1B.3b.
  `mint_l3_validated_golden` (LAST SANCTIONED L3 ORACLE USE) projects L3
  targets per call site into a committed `SemanticGolden` JSON, keyed by
  column-ignoring `GoldenSiteKey` (mirrors `match_sites` strong key; omits
  column because L3 uses UTF-16 cols while fresh uses byte cols).
  `assert_against_semantic_golden` classifies every site into `match`,
  `fresh_wrong`, `fresh_missing`, `fresh_extra`, `fresh_novel`, or
  `golden_missing`; the critical class is `fresh_wrong` (fresh confidently
  resolved to the wrong target — undetectable by Histogram alone).
  `route_applicability` verifies the structural witness↔evidence contract on
  every route and delegates ABI check to `abi_ingestion_integrity`.
  Three new tests: Test 14 (in-repo fixture golden: fresh_wrong=0 and
  fresh_missing=0, regenerable via `REGEN_TEMP_GOLDENS=1`), Test 15
  (route-applicability: violations=0 and abi_unmapped=0 on fixture + env-gated
  CDO), Test 16 (CDO/L3 semantic audit: fresh_wrong ≤ 200 ceiling recorded
  2026-06-30 as 174 — Method/Interface dispatch divergences; deterministic
  SHA-256 digest committed as CDO audit fingerprint).

- **Plan 1B.3a Task 3: Obligation-coverage inventory + `resolve_full_program` + taxonomy'd self-reported metric**
  (`src/program/resolve/full.rs` NEW, `src/program/resolve/mod.rs`,
  `src/bin/aldump.rs`, `tests/program_resolve_harness.rs`,
  `tests/fixtures/full_program_fixture/`) —
  adds `ObligationId` (stable `CallSite` / `Publisher` enum), `Obligation`,
  `ClassifiedEdge`, `Coverage`, `ProgramReport`, `coverage_holds`,
  `is_primary_scope`, `obligation_inventory`, and `resolve_full_program`
  (clean-room, no L3 oracle).  The **COVERAGE CONTRACT** is distinct-id SET
  equality between parsed obligations and classified edges: `coverage_holds`
  fails iff any obligation is silently dropped or any spurious edge appears.
  `--program-call-graph-stats` in `aldump` now prints the whole-program and
  primary-scoped taxonomy'd histograms + coverage + ABI integrity as JSON.
  Three new tests: Test 11 (fixture, 3 call sites + 1 publisher, all buckets
  checked), Test 12 (contract unit: dropped/extra obligation caught), Test 13
  (env-gated CDO gate: coverage holds, `abi_unmapped==0`, primary rate ≤ 7%,
  deterministic across two runs).

### Removed
- **Plan 1B.3b Task 3: remove the L3 oracle (`project_l3*`) from the fresh
  resolver's gates — the engine is now self-validated**
  (`src/program/resolve/differential.rs`, `src/program/resolve/semantic_golden.rs`,
  `src/program/mod.rs`, `src/program/l3_mint.rs` NEW, `src/bin/mint-goldens.rs`,
  `tests/program_resolve_harness.rs`) — deletes the six L3-oracle projection
  functions (`project_l3`, `project_l3_sites`, `project_l3_in_scope`,
  `project_l3_member_in_scope`, `project_l3_implicit_trigger_in_scope`,
  `project_l3_event_rows`) and the four live dual-run "fresh vs L3"
  comparison gates (`run_harness`/`run_site_harness`/`run_resolution_harness`/
  `run_member_resolution_harness`/`run_implicit_trigger_harness`/
  `run_event_flow_gate`, plus their `DiffReport`/`ResolutionReport`/
  `MemberResolutionReport`/`ImplicitTriggerResolutionReport`/
  `EventFlowGateReport` report types) from `differential.rs`. Their coverage
  is now provided entirely by the 1B.3b Tasks 1-2 replacements: the frozen,
  committed, anonymized semantic/trigger/event goldens
  (`run_cdo_semantic_audit`/`run_cdo_trigger_audit`/`run_cdo_event_audit`) +
  `coverage_holds` (Bare/Member), the L3-INDEPENDENT fixture tests
  (`event_fixture_two_stage_join`, `implicit_trigger_fixture_resolves_exact_target_set`),
  and the ported fan-out applicability teeth (`route_applicability`,
  `fan_out_applicability_zero_violations`). The three projections still
  needed to MINT those frozen goldens (`project_l3`,
  `project_l3_implicit_trigger_in_scope`, `project_l3_event_rows`) moved to
  a new module, `src/program/l3_mint.rs` (OUTSIDE `src/program/resolve`) —
  the lone surviving L3-oracle access point in the library, called only by
  the dev-mint tool (`src/bin/mint-goldens.rs`) and the opt-in
  `REGEN_TEMP_GOLDENS=1` fixture-regen test path. `differential.rs` and
  `semantic_golden.rs` now carry ZERO `engine::l3`/`engine::l2` imports; the
  sole remaining `engine::l3` import anywhere under `src/program/resolve` is
  `builtins.rs`'s clean-room `global_builtins` membership-DATA dependency
  (documented as the sanctioned exception). `match_sites`/`SiteMatch`/
  `witness_contract_holds` survive (generic, L3-INDEPENDENT) for their own
  unit tests and `route_applicability`'s witness-contract check respectively.
  `cargo test --workspace` (no `CDO_WS`) fully green on the surviving
  contracts; the frozen CDO audits + `route_applicability` verified green
  and deterministic (run singly with `CDO_WS`+`ENFORCE_CDO_WS=1` — the full
  CDO suite still can't run in parallel, unrelated to this task).

### Fixed
- **(resolve) Split CDO/L3 semantic-audit `fresh_wrong` into adjudicated classes**
  (`src/program/resolve/semantic_golden.rs`, `src/program/resolve/differential.rs`,
  `tests/program_resolve_harness.rs`, `tests/goldens/semantic-edges/known-genuine-divergences.json`) —
  The old `fresh_wrong ≤ 200` ceiling conflated two fundamentally different classes.
  Three-case adjudication in `is_fresh_ahead_dispatch`:
  (1) `l3 ⊆ fresh` — fresh is a superset, more precise;
  (2) all L3 targets are Interface (kind=11) and all fresh targets implement them;
  (3) `fresh ⊆ l3` — fresh partially resolved a compound call (partial-correct, not wrong).
  Result on CDO: `fresh_wrong=174 → fresh_ahead_dispatch=132 genuine_wrong=42`.
  The 42 genuine_wrong are `fresh=builtin (kind=255)` vs `L3=source-routine` **disjoint**
  disagreements on the same callee text — and since the callees are genuine AL builtins
  (`message`/`confirm`/`clear`/`strlen`/`copystr`, `PageInstance::*`/`Record::*`), for most
  of them fresh is **likely correct and L3 is the side in error**; the audit treats L3 as
  the floor by construction, so they land in `genuine_wrong` regardless of which side is
  right (an UPPER bound on fresh errors — confirming the direction is 1B.3b work). All 42
  are enumerated in the committed manifest. Hard gate: `genuine_wrong_count ≤ manifest_count`
  (42) — any NEW disjoint divergence not in the manifest fails CI. fresh_ahead_dispatch (132)
  is always ALLOWED. NOT a clean win.
  `fresh_missing=191` characterization: page_rec=115 codeunit_implicit_rec=24 trigger=38 other=14.
- **(resolve) `witness_contract_holds` made `pub(crate)` in `differential.rs`**;
  duplicate `route_witness_contract_holds` in `semantic_golden.rs` removed — now delegates
  to the single canonical implementation.
- **`resolve_object_run` target-not-found emits `Unknown` (not phantom `AbiSymbol`)**
  (`src/program/resolve/resolver.rs`) —
  the "target not found in any indexed app" arm was constructing an
  `AbiSymbol { app: caller_app_ref, … }` route.  Because the raw ABI index
  only contains dep-app entries (not the workspace app), this caused
  `abi_ingestion_integrity` to report 30 "unmapped" routes.  Fixed to emit
  `RouteTarget::Unresolved + Evidence::Unknown` (honest resolution failure).
- **`build_program_graph` deduplicates `objects` and `routines` after sorting**
  (`src/program/build.rs`) —
  in multi-app workspaces where a sibling app's compiled `.app` lands in
  `.alpackages`, the same source files could be parsed twice (once as
  workspace app, once as embedded dep), producing duplicate `RoutineNodeId`
  entries.  `emit_event_flow_edges` then emitted duplicate publisher edges,
  inflating `histogram.total` by ~60% above the obligation count while coverage
  still held (HashSet de-dup).  Fixed by adding `dedup_by` after `sort_by` for
  both vectors.

- **Plan 1B.3a Task 2: ABI ingestion-integrity invariant + Histogram source/catalog/external split**
  (`src/program/resolve/abi_check.rs` NEW, `src/program/resolve/mod.rs`,
  `src/program/resolve/edge.rs`, `src/program/abi_ingest.rs`,
  `tests/program_resolve_harness.rs`) —
  adds `pub mod abi_check` with `RawAbiIndex` (FRESH re-parse of raw `SymbolReferenceAbi`
  DTOs, independent of `ProgramGraph.routines`), `AbiIntegrityReport`,
  `abi_ingestion_integrity` (per-edge ABI route → raw-index lookup),
  `abi_ingestion_integrity_from_graph` (full-coverage form: checks every SymbolOnly
  `RoutineNode` against the raw index by reconstructing the `AbiRoutineKey` exactly as
  `resolver.rs::make_routine_route` would), and `run_abi_integrity_check` (CDO harness).
  Splits `Histogram.resolved: usize` into `resolved_source` / `resolved_catalog` /
  `resolved_abi_external` (keyed on best-evidence tier across default-firing routes:
  `Evidence::Source` → `resolved_source`, `Evidence::Catalog` → `resolved_catalog`,
  `Evidence::Abi | Evidence::Opaque` → `resolved_abi_external`); `real_unknown_rate`
  unchanged. Makes `object_kind_from_abi_type` and `read_symbol_reference_from_app`
  `pub(crate)`. Five tests: 4 fixture (no env required) + 1 env-gated CDO gate asserting
  `abi_unmapped == 0` and determinism.

- **Plan 1B.3a Task 1: Cached overload-safe ABI ingestion + structured `AbiRoutineKey`**
  (`src/program/abi_ingest.rs` NEW, `src/program/build.rs`, `src/program/node.rs`,
  `src/program/node_extract.rs`, `src/program/resolve/edge.rs`,
  `src/program/resolve/resolver.rs`, `src/snapshot/snapshot.rs`) —
  adds `sig_fp: u64` (FNV-1a fingerprint of param-type sequence) to `RoutineNodeId`
  so same-name overloads with different parameter types are distinct nodes;
  replaces stringly-typed `AbiSymbol { app, symbol_key }` in `RouteTarget` and
  `Witness` with structured `AbiRoutineKey { app, object_type, object_number,
  object_name_lc, routine_name_lc, params_count, param_type_fp, routine_kind,
  event_kind }`; introduces `AbiCache` (process-level `Mutex<HashMap>` keyed by
  `(guid, name, publisher, version)`) and `ingest_abi` which parses SymbolOnly dep
  `.app` SymbolReference.json into `ObjectNode` + `RoutineNode` entries during
  `build_program_graph`; adds `app_path: Option<PathBuf>` to `AppUnit`;
  adds `abi_routine_kind` + `abi_event_kind` fields to `RoutineNode` (always `None`
  for source routines). Four unit tests cover: dep nodes in graph, workspace-only
  graph unchanged, cache-hit across rebuild cycles, local/internal skip.

- **Phase-4b Task 5: Independent event-route teeth + honest framing**
  (`src/program/resolve/differential.rs`, `tests/program_resolve_harness.rs`) —
  adds `verify_event_subscriber_route`: for each fresh EventFlow `Routine` route,
  independently re-reads the subscriber's raw `[EventSubscriber]` `AttributeIr`
  from the `ParsedUnit` IR at gate time (NOT `RoutineNode.event_subscribers`, the
  index's cached parse that built the edge — that would be circular). Checks:
  (1) at least one `[EventSubscriber]` attribute freshly parses to the expected
  `(publisher_object_type, publisher_name, event_name)` triple; (2) subscriber
  `params_count ≤ publisher params_count` (parameter prefix check). FAIL →
  `unverified_extra` (zero-tolerance, asserted 0 in the CDO gate).
  `unverified_extra` is the sixth zero-tolerance gate assertion. Unit tests prove
  non-circularity: passing a `ParsedUnit` with the attribute absent (simulating
  corrupt raw IR) returns FAIL even though the index's cached `event_subscribers`
  would still say PASS — the function demonstrably reads from raw IR.

  **Honest framing (CDO DocumentOutput/Cloud workspace):** on CDO,
  `l3_event_row_count=2` in-scope resolved event rows (CDO is an extension app —
  L3 resolves an event pair only when BOTH publisher and subscriber are
  workspace-indexed source routines; base-app publishers arrive via
  SymbolReference as `AbiSymbol` routes and are not L3-"resolved"). Fresh matched
  both (100% recall of a thin in-scope oracle). The STRUCTURAL coverage —
  arity-FP reconciliation, multiple `[EventSubscriber]` attrs, dispatch conditions
  (Manual/SkipLicense), InternalEvent non-shipping — is carried by the in-repo
  `tests/fixtures/events/` fixture workspace, not the CDO dual-run. `Manual`
  subscribers are conditional `may-edges`; default reachability does NOT traverse
  them. NOT full event-modeling completion: table/page/database trigger-events,
  `BindSubscription` activation, cross-app resolved pairs remain for 1B.3.
  Fixes misleading `l3_sub_lookup` comment: "Stage 1 will still match" is WRONG
  for subscriber-key collisions — reworded to state the real exposure and why it
  is not a problem in practice.

- **Phase-4b Task 4: Structural dual-run event gate** (`src/program/resolve/differential.rs`,
  `tests/program_resolve_harness.rs`, `tests/fixtures/events/`) — adds `run_event_flow_gate`
  with a two-stage arity-FP-reconciled join: Stage 1 = arity-agnostic `EventPairKey`
  set-diff (`pair_l3_only` / `pair_fresh_only`); Stage 2 = within matched keys, arity
  comparison to detect `l3_false_positive_arity_mismatch` (L3 arity-blind last-wins
  picks wrong overload) / `l3_arity_unknown` (accepted) / `l3_regression` (genuine
  disagreement).  Every `pair_fresh_only` is machine-categorized: `l3_maybe_upgrade` /
  `multiple_attr_l3_gap` / `internal_event_non_shipping`.  Five zero-tolerance CDO gate
  assertions: `pair_l3_only=0`, `l3_regression=0`, `fresh_only_uncategorized=0`,
  `fresh_unprojectable=0`, `l3_unprojectable=0` — all pass on CDO.  Fixture workspace
  (`tests/fixtures/events/`) exercises all structural scenarios: overloaded publisher
  (L3 last-wins arity-FP), SkipOnMissingLicense subscriber, multi-`[EventSubscriber]`
  handler (L3 reads only first), InternalEvent subscriber (L3 classifies as "maybe").

- **Phase-4b Task 3: Publisher-anchored `EventFlow` `Multicast` edge emission**
  (`src/program/resolve/resolver.rs`, `src/program/resolve/stub.rs`) — adds
  `emit_event_flow_edges(graph, index, body_map) -> Vec<Edge>`: sweeps all publisher
  event routines in the program graph and emits one `EdgeKind::EventFlow` +
  `DispatchShape::Multicast` edge per publisher, with routes built from
  `ResolveIndex::subscribers_of` (Task 2).  Each route carries the subscriber's
  dispatch conditions (`ManualBinding` / `SkipOnMissingLicense` / …) and a
  `Witness::SourceSpan` (or `AbiSymbol` for SymbolOnly deps).  A publisher with
  zero subscribers emits an empty-routes edge → `classify_obligation` →
  `HonestEmpty`.  Wired into `resolve_program` (stub assembly point); exported from
  `program::resolve`.  Five unit tests cover the manual-binding reachability contract,
  HonestEmpty, non-manual default reachability, and determinism.

- **Phase-4 Task 4: Consolidated Phase-4 fan-out gate + honest scope framing**
  (`tests/program_resolve_harness.rs`) — adds `phase4_fanout_matches_or_beats_l3`,
  a single CDO gate that runs both the member harness (Member + instance-builtin +
  Interface) and the implicit-trigger harness (ImplicitTrigger Multicast) and asserts
  all six zero-tolerance conditions simultaneously: `regression_unexplained=0`,
  `evidence_overclaim=0`, `unverified_extra=0` on each harness, plus the adjudicated
  member divergence cap (≤56).  Prints a unified breakdown separating what Phase 4
  closed from what is explicitly deferred.

  **Phase 4 closes (scoped sub-phase, NOT full spec-§7 whole-program completion):**
  - *Interface Polymorphic fan-out* — `resolve_member` fans out to all known
    implementers; every Routine route is applicability-gated via
    `interface_route_applicable` (method/trigger/kind-level, IR-anchored);
    wrong-overload routes fail → `unverified_extra`; ambiguous overloads →
    `Route{Unresolved, Unknown}` (no guessed route).  `regression_interface=0`
    (drained), `fresh_ahead_interface` routes gate-proven.
  - *ImplicitTrigger Multicast* — `resolve_implicit_trigger` gated vs L3
    `DispatchKind::ImplicitTrigger` oracle; `matched=167`,
    `fresh_ahead_trigger` + `fresh_ahead_validate_fanout` routes applicability-proven;
    empty-target sites → `extra_site` (no triggers on table, benign).
  - *Object/Enum instance-builtins* — CurrPage/CurrReport framework singletons and
    typed-variable Page/Report receivers gated via `instance_builtin_route_applicable`;
    Enum-static dispatch gated via `member_builtin`; `fresh_ahead_instance_builtin=243`,
    `fresh_ahead_enum_static` routes gate-proven; `unverified_extra=0`.

  **Explicitly excluded (honest scope — not claimed as closed):**
  - *EventFlow (Phase 4b)* — deferred: oracle qualification, `ManualBinding`
    property, canonical event key, and reachability honesty for `Manual` subscribers
    (conditional may-edges, not unconditional Multicast) are outstanding; no event
    edges ship to the graph until the qualified oracle gate exists.
  - *Deferred to 1B.3*: `regression_page_rec` (Page/PageExt implicit-Rec
    source-table gap), `regression_compound_receiver` (chained receiver type
    propagation), `regression_codeunit_implicit_rec` (Codeunit TableNo/TestRunner
    implicit-Rec), `trigger.missing_site=78` (L3 ImplicitTrigger sites with no fresh
    peer), and 17 Cat-D divergences (same-object different-procedure overload
    disambiguation).

  Paired-subset results on CDO DocumentOutput/Cloud workspace:
  Member — `matched=7178`, `regression_unexplained=0`, `unverified_extra=0`,
  `verified_win=2790`, `fresh_ahead_instance_builtin=243`, `divergence=56` (cap);
  Trigger — `matched=167`, `regression_unexplained=0`, `unverified_extra=0`.

- **Phase-4 Task 3: ImplicitTrigger Multicast gating** (`src/program/resolve/differential.rs`,
  `tests/program_resolve_harness.rs`) — adds `run_implicit_trigger_harness` comparing fresh
  `resolve_implicit_trigger` (RecordOp sites: insert/modify/delete/validate) against the L3
  oracle filtered to `DispatchKind::ImplicitTrigger`.  Key fixes: L3 callsite_id is the
  `PRecordOperation.id`, not `PCallSite.operation_id` (separate numbering namespace) — built
  direct `op_by_id` map from `L3Routine.record_operations`; callee_fp constructed as
  `"{record_variable_name}.{op}"` to match fresh's raw Member expression text.  Fresh-only
  gating: Validate routes (field=None always fails applicability) classified by table-identity
  check → `fresh_ahead_validate_fanout`; Insert/Modify/Delete routes gate via
  `implicit_trigger_route_applicable` → `fresh_ahead_trigger`; empty-target sites (no triggers
  on table) → `extra_site` (benign).  CDO result on DocumentOutput/Cloud workspace:
  `matched=167`, `regression_unexplained=0`, `evidence_overclaim=0`, `unverified_extra=0`.
- **Phase-4 Task 2: Interface Polymorphic fan-out** (`src/program/resolve/resolver.rs`,
  `src/program/resolve/differential.rs`) — `resolve_member` now implements the
  `ReceiverType::Interface { name_lc }` arm: fans out to all known implementers via
  `ResolveIndex::implementers_of`, resolving each via `resolve_in_object`.  For each
  implementer: SymbolOnly tier delegates directly (arity matching impossible);
  source-tier checks the arity-matched overload count — exactly 1 resolves to a Routine
  route, 0 or >1 emits `Route{Unresolved, Unknown}` (Rule 1: no reachability black hole;
  Rule 2: no guessed route to an ambiguous overload).  Returns `(Polymorphic, routes)`.
  Gate (`run_member_resolution_harness`): added `DispatchKind::Interface` to the L3 oracle
  filter; extended `fresh_combined` to carry site arity and original routes; wired
  `interface_route_applicable` in the FreshOnly handler so every Routine route emitted for
  an interface call is applicability-checked (`fresh_ahead_interface` or `unverified_extra`).
  CDO result on DocumentOutput/Cloud workspace: `regression_interface=0` (drained),
  `unverified_extra=0`, `regression_unexplained=0`, `divergence=56` (cap raised from 45;
  11 new divergences are fan-out sites where fresh emits N targets and L3 emits 1).

### Fixed
- **Phase-4 Task 1: FreshOnly gate discriminator bug** (`src/program/resolve/differential.rs`) —
  The `run_member_resolution_harness` FreshOnly bucketing incorrectly applied the
  `instance_builtin_route_applicable` predicate to ALL FreshOnly sites with non-empty targets,
  not just instance-builtin fan-out routes.  Direct single-dispatch routes (Routine/AbiSymbol
  targets from `resolve_in_object`) were misclassified as `unverified_extra` instead of
  `extra_site`, producing 1223 false `unverified_extra` entries on CDO.  Fix: discriminate
  FreshOnly sites by their canonical target type — routes with `CanonicalTarget::kind=255`
  (Builtin) and `"PageInstance::"` / `"ReportInstance::"` prefix are instance-builtin fan-out
  routes (gate via `instance_builtin_route_applicable` with kind derived from the BuiltinId
  prefix); `"Enum::"` prefix routes are enum-static fan-out (gate via `member_builtin`);
  all other non-empty routes are direct single-dispatch and go to `extra_site`.  Additionally
  handles `Framework(PageInstance/ReportInstance)` receivers (CurrPage/CurrReport singletons)
  by deriving `ObjectKind` from the BuiltinId prefix rather than from the receiver type.
  CDO gate result: `unverified_extra=0`, `fresh_ahead_instance_builtin=243` (3 typed-var
  Object + 240 Framework/CurrPage singletons), `extra_site=1229`, `regression_unexplained=0`,
  `evidence_overclaim=0`, `missing_site=0`, deterministic.

### Added
- **Phase-3 Task 5: Member-resolution gate vs L3** (`src/program/resolve/differential.rs`,
  `tests/program_resolve_harness.rs`) — `run_member_resolution_harness(&Path) ->
  MemberResolutionReport` wires `infer_receiver_type` + `resolve_member` (Tasks 1–4) into
  the dual-run harness for every workspace `CalleeShape::Member` site, then compares against
  the L3 oracle filtered to `PCallee::Member` origin with `dispatch_kind ∈ {Method, Builtin,
  CodeunitRun}`.  Regression bucketing mirrors Phase 2: `regression_interface` (Phase-4
  fan-out), `regression_enum_static` (enum-static deferred), `regression_page_rec`
  (`Record{None}` — Page/PageExt implicit-Rec table gap), `regression_scalar` (Primitive
  by-design), two new named deferral buckets: `regression_compound_receiver` (chained dotted
  receiver e.g. `CurrPage.SubPage.Page` — Phase-4; 47 on CDO) and
  `regression_codeunit_implicit_rec` (Codeunit with `TableNo`/`Subtype=TestRunner` implicit
  `Rec` parameter not captured in IR; 24 on CDO).  CDO gate result (honest paired-subset):
  `regression_unexplained=0`, `evidence_overclaim=0`, `verified_win=2744` (fresh resolved
  2744 sites L3 left empty), `matched=7164`, `missing_site=0` (vs Phase-2 baseline of 3397
  — the capstone metric showing Phase-3 coverage), `divergence=45` (adjudicated: fresh more
  precise than L3 on resolved targets).  Determinism asserted by two consecutive runs.
  `MemberResolutionReport` has 18 fields.
- **Phase-3 Task 3: Object/SelfObject member dispatch** (`src/program/resolve/resolver.rs`) —
  `resolve_member` now handles `ReceiverType::Object{kind, name_lc}` and `ReceiverType::SelfObject`.
  Object dispatch: resolves the target object via `graph.resolve_object`, then calls
  `resolve_in_object` for arity-matched procedure lookup.  Special case: `Codeunit.Run(arity≤1)`
  dispatches to the codeunit's `OnRun` entry trigger (mirrors `resolve_object_run` entry-trigger
  semantics).  SelfObject dispatch: `resolve_in_object` on the calling object itself.
  Both arms produce `Exact` shape with `Source`/`Abi`/`Unknown` evidence matching the target
  tier; OnRun-absent → Opaque boundary route.  Five new unit tests cover all branches.
  Addresses ~800–1200 previously-Unknown member sites.
- **Phase-2 Bare/Run resolution gate vs L3** (`src/program/resolve/differential.rs`,
  `src/program/resolve/resolver.rs`, `src/program/resolve/extract.rs`,
  `tests/program_resolve_harness.rs`, Phase 2 Task 6) — `run_resolution_harness(&Path)
  -> ResolutionReport` wires the real `resolve_bare` / `resolve_object_run` resolvers
  into the dual-run harness and compares against the L3 oracle filtered to in-scope
  dispatch kinds (Direct/Builtin/CodeunitRun/PageRun/ReportRun/Unresolved). New
  `ResolutionReport` struct with 16 fields bucketing: `matched`, `regression_unexplained`
  (gate: 0), `regression_implicit_rec` (deferred), `regression_cross_app` (deferred to
  1B.3 ABI lookup), `evidence_overclaim` (gate: 0), `unverified_extra` (always 0 by
  design; witness quality is covered globally by `evidence_overclaim`), `verified_win`,
  `divergence`, `missing_site`, `extra_site`. Two root causes investigated and fixed:
  (1) AL overloaded procedures share the same `RoutineNodeId` — BodyMap last-write-wins
  stored only one overload's params, causing all other arities to fail → `resolve_in_object`
  now falls back to first candidate when `candidates.len() > 1` (overload signal); (2)
  FreshOnly sites with non-empty targets reclassified as `extra_site` (legitimate
  fresh-only wins from interface-dispatch contexts excluded from the L3 in-scope filter).
  Also added `target_is_name: bool` to `CalleeShape::ObjectRun` and updated `classify_call`
  to use `ExprKind::DatabaseReference` for static ObjectRun target extraction. New
  `is_cross_app_regression` helper documents the dep-boundary SymbolReference gap. CDO
  gate (honest paired-subset result): `regression_unexplained=0`, `evidence_overclaim=0`,
  `unverified_extra=0`, `verified_win=1827`, `divergence=38` (all adjudicated — see
  task-6-report.md), `regression_implicit_rec=90` (Phase 3 deferred). The raw rates
  `fresh_unknown=4.5%` vs `l3_unknown=65.1%` are NOT comparable: denominators differ
  (fresh=4795 in-scope Bare/Run sites vs L3=8196 in-scope edges; `missing_site=3397`
  are L3 Direct/Member-dispatch sites fresh defers to Phase 3) and fresh emits Builtin
  targets while L3 builtin edges carry `to=None`. Honest result: on the paired subset
  (`matched=4304`), fresh has 0 unexplained regressions and 1827 verified wins over L3.
  Whole-branch fix wave added: symmetric paired-subset assertion
  (`total_regressions <= verified_win`), bounded divergence cap (`divergence <= 38`),
  permanent divergence summary print, and honesty comments on `unverified_extra` and
  `is_implicit_rec_regression`. Determinism asserted by two consecutive runs.
- **L3 PCallSite projection + Phase-1 site-parity gate** (`src/program/resolve/differential.rs`,
  `src/program/resolve/extract.rs`, `tests/program_resolve_harness.rs`, Phase 1 Task 4) —
  `project_l3_sites(&Path) -> Vec<CanonicalEdge>` projects every L3 `PCallSite` (not `CallEdge`)
  to a site-level oracle. `run_site_harness(&Path) -> DiffReport` compares fresh structured
  call-site classification (`CalleeShape`) against that oracle and buckets extras into
  `extra_recordop` / `extra_commit` / `extra_implicit_rec` / `extra_unexplained`.
  `extract_sites_for_routine` added to `extract.rs` (per-routine scoping to prevent double-
  counting when multiple same-named triggers exist in one object). Three root causes
  investigated and fixed on the CDO workspace: (1) ancestor `.alpackages` CDO dep with
  identical `AppId` polluted fresh set → `ws_file_set` filter; (2) multi-same-name-trigger
  double-counting → per-routine extraction; (3) report-dataitem-trigger implicit-Rec
  approximation → `dataitem_source_table.is_some()` guard. CDO gate: `matched=13431`,
  `missing_site=0`, `unaligned=0`, `extra_unexplained=0`, `extra_recordop>0`; determinism
  asserted by two consecutive runs.
- **Dual-run differential harness + `aldump --program-call-graph-stats`**
  (`src/program/resolve/differential.rs`, `src/bin/aldump.rs`, Phase 0 Task 7) —
  `run_harness(&Path) -> DiffReport` wires the full pipeline (snapshot →
  ProgramGraph → fresh stub resolve → workspace-scoped canonical projection →
  L3 oracle projection → span-based site matcher → diff buckets). `DiffReport`
  fields: `fresh_total_all_apps`, `fresh_total_workspace`, `l3_edges`, `matched`,
  `regression`, `missing_site`, `extra_site`, `unaligned`. Phase-0 baseline:
  stub resolves nothing → `regression == matched` (all paired sites regress); this
  is the gap Phases 1–4 will close. `aldump --program-call-graph-stats <workspace>`
  prints the `DiffReport` as JSON. CDO gate: `matched > 1000` and `unaligned < 5%`
  confirm the Tasks 4–6 key encodings align on real data; determinism asserted by
  two consecutive runs.
- **L3 → canonical oracle adapter** (`src/program/resolve/differential.rs`,
  Phase 0 Task 5) — `project_l3(&Path) -> Vec<CanonicalEdge>` runs the existing
  L3 resolver over a workspace and projects its `CallEdge`s into the same
  `CanonicalEdge` shape as `project_fresh`, enabling set-diff in the Task 6/7
  harness.  PAnchor line/col are 0-based (same basis as the fresh side);
  columns are UTF-16 vs byte (documented in the function doc, handled by the
  matcher).  Shared helpers extracted: `callee_fp`, `object_kind_str_to_tag`,
  `make_canonical_key` — both projections call these so encodings cannot drift.
  CDO-gated test confirms >1000 edges projected and every site has a real span.
- **CDO whole-program node-graph robustness + app-qualification gate** (`tests/program_graph.rs`) —
  integration test (`CDO_WS`-guarded) that runs `build_program_graph` over the real CDO
  dependency snapshot, asserts panic-free completion, and verifies the resulting graph is
  deep (>500 objects, >2000 routines) and app-qualified (nodes span ≥2 apps) with objects
  deterministically sorted by `NodeId`. On CDO the graph spans 21 apps with 23,432 objects
  and 259,260 routines. Capstone gate for Plan 1B.1.
- **`ProgramGraph` + topology-scoped object index** (`src/program/graph.rs`,
  `src/program/build.rs`) — `build_program_graph(&AppSetSnapshot)` interns all
  apps, extracts object/routine nodes via `parse_snapshot`, wires real dependency
  topology from `declared_deps` (GUID-match preferred, name+version fallback), and
  exposes `resolve_object(from, kind, name)` that searches only `from`'s transitive
  dependency closure — never flat-global. Adds `AppRegistry::find_by_name` helper.
- **Whole-program node graph** (`src/program/`) — app-qualified canonical
  `NodeId`s + topology index over the snapshot (Plan 1B.1). Also adds
  `Hash, Ord, PartialOrd` to `al_syntax::ir::ObjectKind` (plain C-like enum,
  safe and free).
- **Content-addressed source cache** (`src/snapshot/cache.rs`) — `cached_source(app_path)`
  stores the extracted `Vec<SourceFile>` from embedded `.app` packages as
  `<OS-cache-dir>/al-ch-snapshot-cache/<blake3-hex>.json`; the content hash
  is the key so stale reads are structurally impossible. `EmbeddedAppProvider`
  now routes through the cache. `SourceFile` gains `Serialize`/`Deserialize`.
- **Snapshot robustness gate** (`tests/snapshot_robustness.rs`) — `cdo_snapshot_deep_parse_is_panic_free`:
  env-guarded (`CDO_WS`) integration test that builds the full CDO app-set snapshot
  and deep-parses it; asserts no panic and >1000 files parsed (Plan 1A §3.7 gate).
- **App-set snapshot ingestion substrate** (`src/snapshot/`) — per-app source
  acquisition with identity verification + trust tiers (Spec 1 / Plan 1A).
- **`snapshot::parse_snapshot`** — deep-parse of snapshot source into the owned
  IR. `parse_snapshot(&AppSetSnapshot) -> Vec<ParsedUnit>` walks every
  source-bearing `AppUnit` in parallel (local rayon pool, 32 MiB worker stack —
  the `al_syntax` lowerer recurses deeper than the default Windows thread stack
  on large BC packages) and yields `ParsedUnit { app, files: Vec<ParsedFile> }`
  holding the owned `al_syntax::ir::AlFile` per source file. Symbol-only boundary
  units contribute no output; their ABI feeds later resolution.

### Changed
- **Pinned the toolchain (`rust-toolchain.toml` → 1.96.0).** CI floated `dtolnay/
  rust-toolchain@stable` while gating on `cargo clippy -- -D warnings`, so every new
  clippy release that adds lints could break CI with no code change (it did: 1.96 added
  `unnecessary_sort_by` / `useless_conversion` cases the 1.94 dev box never saw). The pin
  makes CI deterministic and matches local dev: `ci.yml` keeps `dtolnay/rust-toolchain@
  stable` (a base install with rustfmt/clippy), but every `cargo` command runs under the
  toml-pinned version via the rustup override, so the file is the single source of truth.
  Bump deliberately + clear new lints in the same PR. Also fixed the 1.96 lints surfaced:
  3 `sort_by` → `sort_by_key(Reverse(..))`
  (descending sorts preserved), 2 redundant `.into_iter()` in `chain(..)`.
- **Cleared the clippy `-D warnings` debt + whole-crate edition-2024 rustfmt** (CI gate
  prerequisites for merging `feat/owned-syntax-ir` → `master`). The edition-2024 upgrade
  enabled let-chains, so clippy's `collapsible_if` flagged ~155 `if x { if let … }` nests
  (master @ 2021 never saw these); `cargo clippy --fix` collapsed them to let-chains.
  Remaining handled by hand: 2 `never_loop`s (`for f in … { return Err }` → `if let
  Some(f) = …next()`), `strip_prefix`/`clamp`/`from_ref`/`&Path`/`needless_range_loop`/
  `redundant_guard` rewrites, doc-list indentation, and `#[allow]` with rationale for the
  inherent ones (`too_many_arguments` on document-envelope builders, `type_complexity` on
  parallel index maps, `large_enum_variant`, `enum_variant_names` where `Event` is the AL
  domain term). ~22 dead-code items (telemetry `dedup` module, detector `INVALIDATING_OPS`,
  `is_edge_kind`, never-read data-model fields, etc.) were triaged as future-design
  scaffolding and kept under targeted `#[allow(dead_code)]` with notes — none were obsolete.
  Then a one-time `cargo fmt` normalized the 277 stale edition-2021-formatted files (the
  per-file `rustfmt` hook keeps them clean afterward). `cargo clippy --release -- -D
  warnings`, `cargo fmt --check`, and `cargo test --workspace` all green.

### Fixed
- **Deterministic dependency order + GUID-then-name topology matching.**
  `load_all_apps` now sorts its output by the AppId 4-tuple (GUID, name, publisher,
  version) before returning, making `AppRef`/`NodeId` numbering reproducible across
  machines and filesystems (charter C8). Topology wiring in `build_program_graph`
  previously fell through to name+version only when the dep carried no GUID; it now
  tries GUID first and falls through to name+version when the GUID match yields
  `None` — closing the gap where a dep carries a GUID but the matching snapshot unit
  has an empty `id.guid`.
- **Dependency apps now carry their real unique GUID (and publisher).** `AppMetadata`
  parsed only `name`/`version` from `NavxManifest.xml`, dropping the `App@Id` (the app's
  only globally-unique identity) and `Publisher` — so `SnapshotBuilder` built dependency
  `AppId`s with `guid: ""`, leaving cross-app node identity leaning on name+version
  uniqueness. `parse_manifest` now also extracts `Id` → `AppMetadata.app_id` and
  `Publisher`, and the dependency `AppId` is built from the `.app`'s authoritative manifest
  (the workspace already read its own `id` from `app.json`). Local-provider matching now
  prefers GUID when known. The identity foundation Plan 1B builds on is now truly unique.
  The same manifest-enrichment pass fixes two more workarounds: (a) dependency `AppUnit`s
  now carry a REAL compilation basis (`Runtime`/`Platform`/`Application` from the manifest)
  instead of an empty `CompilationContext::default()` — note the source-level `#if`
  preprocessor symbols are still NOT recoverable from a `.app` (that needs SymbolReference
  reconciliation, a later phase); (b) `AppMetadata` + every `AppUnit` now carry the app's
  **declared dependencies** (each with its GUID, from the manifest `<Dependencies>` /
  app.json), so Plan 1B's resolution can be dependency-topology-aware instead of flat-global.
  `AppDependency` gains `app_id` (parses the app.json / manifest `id`).
- **Member-trigger names (`Object::Member`) were truncated to the object half.** The
  grammar's `_trigger_name` was an inlined `seq(id, '::', id)`, so the `name` field of
  `trigger_declaration` was `multiple:true` and included the anonymous `::` token; the
  lowerer's `field("name")` returned only the FIRST node (`UserTours`), silently dropping
  `::ShowTourWizard`. Introduced a named `member_trigger_name` node (`object` / `member`
  fields) so `name` binds a single value (`multiple:false`, no `::` in its type set), and
  the lowerer now joins it to the full qualified `Object::Member` name. Grammar issue #4
  closed. (No member triggers in the test corpus → zero golden divergence; +1 named kind
  → 388, new node-types hash `90f25499…`.)

### Changed
- **tree-sitter-al grammar: case-pattern field-pollution cleanup.** Case branches no
  longer leak spurious fields. Two grammar-level root causes, both fixed in the owned
  grammar (`tree-sitter-al` submodule):
  1. `field('pattern', $._case_pattern)` wrapped an *inlined* `repeat` whose members
     included the `,` separators, so the `pattern` field distributed over the comma
     tokens — `children_by_field_name("pattern")` returned anonymous `,` nodes and the
     owned-IR lowerer panicked on `case 1, 2:`. Introduced `_case_pattern_item =
     seq(field('pattern', $._single_pattern), optional(','))` so the `pattern` field
     binds a single value node, never a separator. `case_branch`,
     `preproc_split_case_branch`, `preproc_split_case_extended`, and
     `preproc_conditional_case_patterns` all consume `_case_pattern_item`.
  2. The `in`-as-case-pattern arm was an inline `seq(field('left',…), field('operator',
     …), field('right',…))` inside `_single_pattern`, so `left`/`operator`/`right`
     leaked onto every case node. Replaced with the existing named `$.in_expression`;
     the now-unnecessary `[$._single_pattern, $.in_expression]` conflict was removed.
  Net effect on `node-types.json`: −876 lines of field pollution; named-kind count
  unchanged at 387 (`_case_pattern_item` is inlined, `in_expression` already existed).
  The lowerer's defensive `is_named()` filter is kept as defense-in-depth. Regenerated
  the raw vocab (`gen-syntax`, new node-types hash `8f9b7013…`). Zero al-sem differential
  divergence. (Reviewed: gpt-5.5 + gemini-3.1-pro.)
- **Upgraded to Rust edition 2024** (from 2021) across all three crates — it is 2026 and
  edition 2024 is the current stable (rustc 1.94). `cargo fix --edition` applied the
  migrations: `unsafe extern "C"` (the al-syntax grammar FFI), `unsafe { std::env::set_var
  / remove_var }` (now unsafe in 2024 — a real parallel-test environment race the edition
  surfaces), and an over-conservative `if let/else`→`match` rewrite (tidied back to
  `if let … else`). Added a workspace `rustfmt.toml` with `edition = "2024"` as the SINGLE
  source of truth — `gen-syntax` and the editor `rustfmt` hook no longer hardcode an
  edition. Full `cargo build`/`test --workspace` green under 2024.

### Fixed
- **`raw_kind_round_trips` stale assertion** — it pinned `NAMED_KIND_COUNT == 386`, but
  the generated const is `387` (the `call_statement` grammar node added a named kind;
  the const regenerated, the test literal did not). Went unnoticed because root
  `cargo test` doesn't run member-crate tests without `--workspace`. Fixed to 387; run
  `cargo test --workspace` going forward.

### Changed
- **`gen-syntax` now rustfmts its generated Rust output** (`raw_kind.rs` / `field.rs` /
  `nodes.rs` / `mod.rs`), so the checked-in generated code is canonical AND stable across
  regenerations — a developer's `cargo fmt` produces the same bytes the generator does
  (no fmt/gen-syntax ping-pong). Mirrors how rust-analyzer formats its ungrammar-
  generated syntax nodes. Recommended CI guard: `cargo run -p xtask -- gen-syntax &&
  git diff --exit-code`. (Reviewed: gpt-5.5 + gemini-3.1-pro.)

### Added
- **Serde-skip drift gate.** The IR L2 feature snapshot (`tests/ir_l2_snapshot.rs`) now
  digests the `Debug` representation of each routine's `PFeatures` instead of serde
  JSON, so it covers the `#[serde(skip)]` (and `PartialEq`-excluded) fields a serialized
  golden cannot see — `PRecordOperation.in_until_condition` / `run_trigger`,
  `PCFNNode.source_range` / `is_case_else`, `PVarAssignment.rhs_identifier`. Four such
  load-bearing fields silently broke during the migration because the old byte gate
  (serde + PartialEq) was blind to them. A `debug_digest_catches_serde_skip_drift` proof
  test demonstrates the blind spot (two ops differing only in `in_until_condition`
  serialize identically and compare equal, yet their Debug digests differ).
- **Parenless statement calls are now call-hierarchy edges.** `parse_file_ir` captures
  every `ExprKind::Call`, including the parenless forms (`Initialize;`, `Rec.Find;`,
  `Modify;`) the old `call_expression`-only query missed. A procedure invoked only as
  `MyProc;` is now a real incoming/outgoing call edge and no longer mis-flagged as
  unused; parenless record builtins simply don't resolve to a user procedure. (Deferred
  completeness fast-follow from the Phase 4 zero-diff port.)
- **Grouped variable declarations yield every name.** `A, B: T` now produces a variable
  for BOTH `A` and `B` (the old query captured only the first, leaving trailing names as
  untracked receivers / false unknowns). Quoted grouped names are handled too.

### Removed
- **The engine's `tree-sitter` dependency is gone — `al-syntax` is the SOLE
  tree-sitter linker (Phase 5 SEAL complete).** Deleted the test-only legacy L2
  "dual-run oracle" (`dual_run_support.rs`, `tests/ir_dual_run.rs`) and the legacy
  tree-sitter L2 body-walk (`engine/l2/{body_walk,cfn,classify}.rs` + the tree-sitter
  fns in `mod`/`scope`/`node_util`/`control_context`/`operation_order`/`l2_workspace`),
  keeping the tree-sitter-free production helpers. Removed `tree-sitter` +
  `streaming-iterator` from `[dependencies]`. The engine consumes `al_syntax::parse`
  exclusively; `cargo tree -i tree-sitter` now shows only `al-syntax`.
  - The L2 single-routine analyzers (`control_context::analyze_named_routine`,
    `operation_order::analyze_named_routine_order`) + the `features_for_named_routine`
    test entry now build `PFeatures` via the owned IR
    (`l2_workspace::ir_features_for_named_routine`); the l2 / l2cc / l2order vector +
    oracle tests and `temp_state_capture` were converted to the IR path (no tree-sitter).
  - The migration-era `tests/ir_object_set_parity.rs` (IR-vs-tree-sitter set parity, a
    Phase-2/3 cutover precondition) is retired — its invariant is permanently satisfied.
  - Rebaselined 2 synthetic L2 vectors: the IR no longer emits an UNQUOTED qualified-enum
    VALUE (`Codeunit::A` → `a`) as a `condition_reference`. The legacy capture was a
    tree-sitter token-shape artifact (it captured a bare `identifier` but never a quoted
    value); an object/enum name is a compile-time constant, not a runtime variable, so
    dropping it is more accurate (reviewed: gpt-5.5 + gemini-3.1-pro). No production
    golden impact (the corpus's only such case is quoted).

### Changed
- **R0 identity snapshot (`engine::snapshot` / `aldump`) now derives from the owned IR**
  (`al_syntax::parse`) instead of its own tree-sitter walk (Phase 5 step). Object/
  routine identity (stable ids, signature fingerprints, normalizedSignatureHash,
  canonicalSignatureText) reuses the shared `engine::ids` algorithms, so R0 identity
  equals production identity. Byte-identical to the prior output — the R0 goldens pass
  unchanged. Removed `extract_from_tree` + the tree-sitter object/routine/param walkers.
- **`workspace_diagnostics` "No object declaration found" now uses the owned IR**
  (`al_syntax::parse(...).objects.is_empty()`) instead of a direct tree-sitter
  root-children scan (Phase 5 step). The diagnostic now matches exactly what the
  engine indexes (including objects nested under a `namespace`, which the old
  direct-child check missed). Removed the tree-sitter `Parser` + `root_has_object_declaration`.

### Removed
- **The legacy tree-sitter LSP parser is gone (Phase 4 complete).** Deleted `AlParser`
  + the 6 S-expr queries' consumers in `parser.rs`, the tree-sitter
  `analysis::calculate_complexity`, and the legacy CST metric walk in `main.rs`. The
  entire LSP front-end (parser / handlers / indexer / analysis / CLI metrics) now runs
  on the owned `al-syntax` IR. The AlParser differential is replaced by a forward
  digest snapshot golden of `parse_file_ir` over the r0-corpus
  (`tests/parser-ir-goldens/projection.snapshot`, regen via `REGEN_TEMP_GOLDENS=1`);
  the parser unit tests now exercise `parse_file_ir`.

### Fixed
- **`al_syntax::parse` no longer panics on a multi-value `case` branch.** tree-sitter-al
  v3 tags the `,` separators between a case branch's values with the `pattern` field, so
  `children_by_field(Pattern)` returned anonymous `,` tokens; lowering one as an
  expression hit `RawKind::from_raw(",")` and panicked ("unknown node kind") — a real
  crash reachable from the production parser on real BC code (e.g. `SalesPost`). The
  case-pattern lowering now filters to named nodes (added `RawNode::is_named`).

### Added
- **IR-owned L2 feature snapshot gate (`tests/ir_l2_snapshot.rs`).** Serializes the
  full `PFeatures` (loops / ops / record-ops / calls / field-accesses / record-vars /
  nesting / branching / unreachable / identifier+condition refs / variables /
  var-assignments / the `statement_tree` CFN) of every r0-corpus routine via
  `project_routine_features_ir`, digested into `tests/ir-l2-goldens/l2_features.snapshot`
  (REGEN with `REGEN_TEMP_GOLDENS=1`). This is the deepest L2 contract as a Rust-OWNED
  baseline — it replaces the migration-era legacy-vs-IR dual-run oracle without
  ossifying against the deleted tree-sitter walk.
- **`al_syntax::lookup_symbol_properties` facade (Phase 4, step 3).** A semantic,
  owned-types CST-backed lookup for a table field's / page action's properties
  (`SymbolDeclKind`, `SymbolProperties`). The IR models a field's number/name/type/
  class but not arbitrary per-field properties, and doesn't model actions — so these
  two niche LSP requests (`fieldProperties` / `actionProperties`) call this facade
  rather than bloating the always-parsed IR. tree-sitter stays inside `al-syntax`; no
  `tree_sitter` type crosses the boundary.
- **Owned-IR projection of the LSP front-end `ParsedFile` (Phase 4, step 1).**
  `parser::parse_file_ir(source)` produces the same `ParsedFile` (definitions / calls /
  variables / event subscribers+publishers / framework-invoked / object) as the legacy
  tree-sitter `AlParser`, but sourced entirely from `al_syntax::parse` — no S-expr
  queries. It is the ZERO-DIFF projection: it deliberately reproduces the legacy query
  set (`call_expression`-only calls, first-name-only multi-name vars, the legacy
  object-kind coverage), proven byte-identical to the legacy parser across all 335
  in-repo r0-corpus files by a new differential unit test
  (`ir_projection_matches_legacy_over_r0_corpus`). Correctness gains the IR enables
  (parenless statement calls, all multi-name vars) are deliberate fast-follows.
- **`RoutineDecl.name_origin`** (al-syntax IR): the origin of the routine's NAME
  identifier (vs the whole-routine `origin`), for an LSP call-hierarchy item's
  `selection_range` (e.g. an event publisher's procedure-name range).

### Changed
- **LSP front-end production paths now run on the owned IR (Phase 4, step 3).**
  `handlers::field_properties`/`action_properties` call the al-syntax facade;
  the CLI `--analyze` per-procedure metrics (`main::extract_metrics_ir`) iterate the
  IR and use the canonical IR cyclomatic-complexity walker
  (`parser::routine_complexity_ir`); `analysis`'s complexity unit tests assert against
  that IR walker. The tree-sitter `analysis::calculate_complexity` + the legacy
  `AlParser` (and its 6 S-expr queries) remain ONLY as the differential-test oracle
  behind `#[allow(dead_code)]`, deleted next (Phase 4.4) when the differential becomes
  an IR-output snapshot golden.
- **L3 is now tree-sitter-free (Phase 3 complete).** `l3_workspace::project_file` no
  longer takes a tree-sitter `root` — it iterates the owned IR directly
  (`ir_file.objects` → `o.routines`), sourcing every routine's kind / attributes /
  access / body / params / return / norm-hash / source-anchor / cc-params /
  entry-temp-guard / enclosing-member from the IR. Both callers
  (`assemble_workspace` / `assemble_workspace_units`) stopped creating a tree-sitter
  `Parser` and parsing source; the IR (already produced once upstream) is the sole
  input. The IR routine set is byte-identical to the former tree-sitter routine set
  (591/591 on the corpus, malformed routines included), so the iteration switch is a
  zero-golden-change refactor. Removed ~560 lines of now-dead legacy CST extractors
  (`extract_object_name`, `index_table`, `collect_routine_nodes`, `enclosing_member_of`,
  the body-guard matchers, …); l3_workspace.rs is warning-clean.
- **L3 object & table metadata are now owned-IR-driven.** `l3_workspace::project_file`
  sources object name/number, properties (SourceTable/PageType/Subtype/
  InherentCommitBehavior/SourceTableTemporary/TableNo), `extends` target,
  `implements` interfaces, page controls, and table fields/keys/TableType from the
  owned IR (matched by start byte; legacy tree-sitter extractors only as a defensive
  fallback). New IR: `ObjectDecl.{extends_target, implements, page_controls, fields,
  keys}` + `PageControl` / `FieldDecl`. Validated byte-identical via the L3 goldens.
  (Residual tree-sitter in L3: per-routine params/attrs/kind/access metadata, object
  globals, and two body-pattern guards — `entry_temp_guard` + the table temp-contract
  `IsTemporary` guard — still walk the CST; next increment.)
- **L3 routine features are now owned-IR-driven (the last production `body_walk`
  caller is gone).** `l3_workspace::project_file` sources each routine's `PFeatures`
  from `project_routine_features_ir` (matched by start byte; a defensive legacy
  fallback only on a corpus-impossible byte-miss). The legacy `body_walk` /
  `project_routine_features` now survive ONLY as the dual-run validation oracle.

### Fixed
- **IR CFN nodes carry `source_range`** (was always `None`). The L4 branch-aware
  field-load walker reads this serde-skipped field to attribute field accesses to the
  right block level; without it, the walker reconstructed a too-narrow range from
  op/callsite leaves only and dropped statement-level field reads — diverging the L4
  cross-call `requiredLoadedFieldsAtEntry` / `dirtyAtExit` summaries. Now populated
  from each statement/block/branch IR origin, byte-identical to the legacy `cfn.rs`.
- **`RecordRef` / `RecordId` are no longer misclassified as `Record` variables.** The
  IR's record-variable test used `type.starts_with("record")`, which wrongly matched
  the distinct `RecordRef` type — seeding its record ops a spurious `Known(false)`
  temp_state via the backfill. The record-VARIABLE test now requires `Record`
  followed by whitespace/`"` (or exactly `Record`); the record-OP RECEIVER set stays
  inclusive (so `RecRef.DeleteAll` is still captured as a record op, as in legacy).

### Added
- **tree-sitter-al `call_statement` grammar node + engine integration.** A parenless
  no-arg call (`Initialize;`) — a bare identifier in statement position that owns its
  `;` — now parses as a `call_statement` node, structurally distinct from an
  ERROR-recovery bare identifier (which has no terminator and stays raw). This lets the
  owned-IR lowerer capture parenless procedure calls as call-graph edges WITHOUT
  mistaking parse-error debris for a call (the moat-polluting case). The IR lowerer
  lowers `call_statement` to a parenless Call (anchored on the callee identifier so the
  source anchor is byte-identical to the pre-grammar form); a bare identifier in
  statement position is treated as debris / semicolon-less and is NOT a call. The legacy
  tree-sitter walks (the dual-run oracle + the L3 emitter) treat `call_statement`
  transparently (unwrap to the function child), preserving byte parity. Grammar
  designed + reviewed with gpt-5.5 + gemini-3.1-pro; parenful `Foo()` and parenless
  member `Rec.Find;` are unchanged. Known residual: a parenless call written WITHOUT a
  trailing `;` (a semicolon-less final statement, rare) is not captured — never a false
  edge, and no worse than the legacy walk which captured no parenless calls at all.
- **Report dataitems modelled in the owned IR.** `ObjectDecl.report_dataitems`
  (`(name, source-table)` pairs) and `RoutineDecl.dataitem_source_table` (a dataitem
  trigger's implicit-`Rec` table) let the IR-driven L2 path seed a report dataitem
  trigger's implicit `Rec` (typed to its enclosing dataitem's source table) and the
  dataitem-name record vars across all the report's routines — parity with the legacy
  `report_dataitem_source_table` / `report_dataitem_record_vars`. Nested dataitems use
  innermost-wins (None when the innermost dataitem's table is absent, matching legacy).

### Changed
- **L2 emitter is now fully owned-IR-driven — no tree-sitter CST walk.**
  `l2_workspace::project_file` and `project_named_routine` iterate the owned AL
  syntax IR (`al_syntax::parse`) directly: objects, routines, metadata, parameters
  and per-routine `features` all come from the IR, and `project_workspace` no longer
  parses tree-sitter at all. Preconditions proven over the r0-corpus before cutover
  (object set 404/404, routine set 591/591, `(type,number,name)` 404/404,
  `parse_incomplete` 591/591); feature output is byte-identical to the legacy
  body_walk on every well-formed routine. `project_named_routine` dropped its
  `tree: &Tree` parameter. Added `al_syntax::ir::RoutineDecl.parse_incomplete` and
  `ir_walk::ir_object_type` to support the cutover.

### Fixed
- **Malformed-routine statementTree no longer carries a phantom `other` node.**
  The legacy tree-sitter ERROR-recovery emitted a spurious `{kind:"other"}`
  statement_tree child for a stray token inside a body; the IR cleanly drops the
  ERROR token. Rebaselined the one affected Rust-owned golden
  (`ws-callsite-resolutions`).

## [0.9.3] - 2026-06-26

The tree-sitter-al v3 compliance work. (v0.9.1 and v0.9.2 were tagged during the
migration; the new release test gate correctly blocked both before publishing
any binaries — v0.9.1 on the engine port, v0.9.2 on a CI-only test-harness gap —
so this is the first published v3-compliant build.)

### Fixed
- **cli-b diff differential tests are CI-safe.** They byte-compare against
  goldens in the sibling al-sem repo (`AL_SEM_DIR`, default `U:\Git\al-sem`) and
  previously panicked when that checkout was absent. They now skip when the
  goldens are not present — matching `al2dump_smoke` — so the release test gate
  (which has no al-sem) passes while dev machines still run them as the safety net.
- **Enriched-hover field/action property extraction broken against tree-sitter-al
  v3.** v0.9.0 was built by CI against the grammar repo's default branch, which had
  advanced to v3.0.0+ where a declaration's properties/triggers are wrapped in a
  `body` field (a `declaration_body` node) instead of being direct children.
  `extract_all_properties` only iterated direct children, so `al-call-hierarchy/fieldProperties`
  and `al-call-hierarchy/actionProperties` (the enriched-hover backend) returned no
  properties. It now descends into the `body` field when present, with a fallback
  to direct children for older grammars.
- **`object_body` node rename.** tree-sitter-al v3 renamed `object_body` to
  `declaration_body`; the L3 workspace name-walk now accepts both so it still stops
  at the declaration body boundary.
- **Full L2/L3 traversal port to the v3 node shapes.** v3 inserts wrapper nodes
  that broke every flat (direct-child) traversal while recursive walks kept
  working. All affected sites now descend the wrappers, restoring byte-identical
  L2/L3 projections (the R0/R1a differential goldens pass with zero divergences):
  - **statements** — a `code_block`'s statements (and a `repeat`/case-branch body)
    are nested in a `statement_block`. A shared `block_statements` helper flattens
    it inline (preserving trailing trivia order). Fixes the L5 transaction
    detectors that reported **zero** candidates (d40 transitive-load, d46
    commit-in-lifecycle, d47 io-unsafe-txn, d49 uncommitted-write-before-ui, d51
    retry-side-effect), the CFN `statementTree`, unreachable-statement detection,
    and the temp-table guard scan.
  - **case branches** — wrapped in a `case_body`; the CFN builder now reads
    branches from it (the `case_else_branch` stays a direct child).
  - **object properties** — `Subtype`/`SourceTable`/`FieldClass` live under
    `declaration_body`; object-property and field-class reads descend it.
  - **object-global var sections** — nested in `declaration_body`; global record
    variable extraction descends it.
  - **statement-position calls** — a parenless method call's parent is now the
    `statement_block`; `is_pure_statement_parent` accepts it, so calls like
    `Customer.SetRecFilter;` and `with`-receiver `Modify` are no longer mis-read as
    field accesses / dropped.
  - **object-run result-consumed** — a bare call statement's parent is the
    `statement_block`; classified as not-consumed like the old `code_block` case.
  - **member-trigger enclosing member** — a field/action/dataitem trigger's parent
    is now a `*_body` wrapper (declaration_body / report_body / ...); resolution
    steps up through it to the named member, while object-level triggers (OnRun)
    stay member-less.

### Changed
- **Grammar compliance with tree-sitter-al v3.0.1.** Source now builds and passes
  the full test suite against the v3 grammar (the `tree-sitter-al` submodule is
  updated to v3.0.1). CI builds against the grammar's default branch, so this keeps
  the source compliant with the latest parser.

### CI
- **Release pipeline now runs the test suite as a prerequisite.** `release.yml`
  gained a `test` job (`cargo test --release --all-targets`) that both build jobs
  depend on, so a tag whose tests fail against the grammar produces no binaries and
  no GitHub release. This closes the gap that let v0.9.0 ship the broken hover.

## [0.9.0] - 2026-06-26

### Changed
- **tree-sitter-al bumped to v2.6.0 (`cddeb82`).** Clean upgrade from v2.5.2-shim
  (`89b1d05`): it parses the full BC repo set (not just the base app) via new additive
  node kinds for construct-internal preprocessor patterns (`preproc_pragma_only`,
  `preproc_conditional_{option_members,labels,rendering}`, `analysisviews_section`,
  `ternary_expression`, `preproc_split_if_then_begin_else_shared`). Unwrapped code parses
  byte-identically, so engine queries needed no change. Cross-app resolution is unchanged
  on CDO (4 unknown / 13689 = 0.029%) and resolves slightly MORE on DC (resolved
  18791→19103, unknown flat at 83 / 0.252%). All cli-a detector findings/evidence/factIds
  and the (source-only) workspace fingerprint are unaffected by the grammar.

### Fixed
- **Implicitly-invoked procedures no longer flagged `unused-procedure`**
  ([al-lsp-for-agents#20](https://github.com/SShadowS/al-lsp-for-agents/issues/20)).
  Local procedures were always tagged `DefinitionKind::Procedure` and the
  `[EventSubscriber]` attribute was parsed into a separate list that never
  updated the definition's kind, so the unused-procedure exclusion was dead code
  for workspace subscribers. Subscribers are now reconciled to
  `DefinitionKind::EventSubscriber`, and an audit-surfaced class of related false
  positives is excluded too: `[Test]` methods, test handlers (`[ConfirmHandler]`,
  `[MessageHandler]`, `[PageHandler]`, ...), and public event publishers
  (`[IntegrationEvent]`/`[BusinessEvent]`, whose subscribers live in downstream
  apps that aren't loaded). `[InternalEvent]` publishers stay flagged when
  orphaned — they can only be subscribed within the same app, so an unused one is
  genuine dead code. Tracked per file in a new `implicitly_invoked` set cleared in
  `remove_file` alongside the definitions. Validated on real Document Output
  source: removes 21 false-positive public event publishers in one app while
  still flagging real dead procedures.
- **`.gitattributes`: force `eol=lf` on `tests/**/*.md` goldens.** The gate PR-summary
  (`*.prsummary.md`) and r0 goldens are byte-compared, but `*.md` lacked the `eol=lf` rule
  its `*.json`/`*.sarif`/`*.txt`/`*.html` siblings already have, so on a
  `core.autocrlf=true` checkout they materialized as CRLF and byte-mismatched the LF engine
  output (`gate_prsummary_differential`, `gate_suppress_baseline_differential`). Added the
  missing rule to match the existing pattern.
- **`.gitattributes`: force `eol=lf` on `tests/**/*.html` goldens.** The cli-a html
  differential goldens are byte-compared, but `*.html` lacked the `eol=lf` rule its
  `*.json`/`*.sarif`/`*.txt` siblings already have, so on a `core.autocrlf=true` checkout
  they materialized as CRLF and byte-mismatched the LF engine output. Added the missing
  rule to match the existing pattern.
- **Cloud-review remediation (engine-d22 branch review).** Three findings fixed:
  - `compound_call_result_receiver` validated text before the call's `(` but not after its `)`,
    so `GetCustomer().Name` (receiver of `GetCustomer().Name.Trim()`) was mis-typed as
    `GetCustomer`'s return type, silently dropping the trailing `.Name` — a false resolution.
    Now balance-walks from the first `(` to its matching `)` and declines unless that `)` is the
    final char (accepts arg-list dots/nesting like `Func(a.b, G(x))`; rejects `Func().Field` /
    `Func().Other()`). Regression test added.
  - `compound_receiver_shape` truncated the diagnostic tag with a raw `[..120]` byte slice, which
    panics when byte 120 is not a UTF-8 char boundary (localized AL identifiers are non-ASCII).
    Now floors to a char boundary — honors the "engine never panics" contract.
  - `extract_record_variables` (local record vars) still scanned only direct `var_section`
    children, so a `#if`-guarded local record var was missed while the object-global paths
    (fixed earlier) were not. Now uses `var_section_declarations`, mirroring them.
- **Preprocessor-guarded object globals are now extracted.** A global variable declared inside a
  `#if`/`#else` block in a var section — `var #if BC24 NoSeriesMgt: Codeunit "No. Series" #else
  NoSeriesMgt: Codeunit NoSeriesManagement #endif` (ubiquitous in BC version-compat code) — was
  invisible to both object-global extractors (scalar + record), which only scanned direct
  `var_section` children and skipped the `preproc_conditional_var` wrapper. Every member call on
  such a global (`NoSeriesMgt.GetNextNo(...)`) degraded to `Unknown{UntrackedReceiver}`. A new
  `var_section_declarations` helper descends through the preprocessor wrappers; same-name branches
  are de-duplicated first-wins (mutually exclusive at compile time). DC deps-loaded:
  realUnknownRate 0.304% → 0.252% (unknown 100→83).

### Added
- **`Version`/`File` static receivers + `CompanyProperty`/`SessionInformation` singletons.**
  `Version.Create(...)` and `File.Exists(...)`/`File.Open(...)` now resolve via the static-type
  interception (File/Version value-type catalogs); `CompanyProperty.DisplayName()` and
  `SessionInformation.*` resolve via the Step-2c singleton interception (new `CompanyProperty`
  framework kind with its 3-method catalog; `SessionInformation` kind already existed). DC
  deps-loaded: realUnknownRate 0.337% → 0.304% (unknown 111→100).
- **`this.OwnMethod()` self-instance calls resolve.** A bare `this` receiver (the modern-AL
  self-instance qualifier, e.g. `this.CTSCDNUpdateeDocumentStatus(...)` in a PageExtension) now
  types as the new `ReceiverType::SelfObject` and dispatches the method among the CALLER routine's
  own object's procedures (by `object_id`) — so it resolves for ANY object kind, including
  PageExtension/TableExtension that have no `ObjectKind` variant. The object-dispatch resolution
  tail was factored into a shared `resolve_method_in_object` helper. DC deps-loaded:
  realUnknownRate 0.36% → 0.337% (unknown 118→111).
- **Enum/option VALUE references (`::`) resolve as enum receivers.** An enum member-access
  expression used as a receiver — `Rec."Document Type"::Order.AsInteger()`,
  `Enum::"CDC Translate To Type"::Item.AsInteger().ToText()`, `EMailLog."Linked to Table"::Customer.AsInteger()`
  — now types as `Framework{Enum}` so `.AsInteger()`/`.Ordinals()`/`.Names()` classify `builtin`.
  The `enum_receiver` helper (generalized from the prior `Enum::`-only handler) covers the
  static-type, type-value, and field-value forms; object-ID `::` refs (`Codeunit::"X"`,
  `Page::"X"`, …) are excluded (they yield Integer, not enum). `framework_method_return_type`
  now maps Enum `AsInteger` → Integer so the `.AsInteger().ToText()` chain resolves. Big win on
  document-type-heavy code: **DC deps-loaded realUnknownRate 1.00% → 0.36% (unknown 330→118)**;
  CDO 0.037% → 0.029%.
- **Enum type NAME as a static receiver.** A bare/quoted identifier that names an Enum object,
  used as a receiver — `"CDO Send on Posting".FromInteger(x)`, `MyEnum.Names()` — now types as
  `Framework{Enum}` (resolved via a symbol-table `object_by_type_name("Enum", …)` lookup), so its
  static methods classify `builtin`. A real variable of the same name shadows it. CDO deps-loaded:
  untracked-receiver 2→1, realUnknownRate 0.044% → 0.037%.
- **Text/Code table fields resolve as Text receivers; field-kind resolution unified.** A
  Text/Code-typed table field used as a member receiver — `"Azure Blob Private Endpoint URL".Trim()`
  (implicit Rec), `CollectedErrors."Additional Information".Contains(...)` (declared record) —
  now types as `Framework{Text}` so its Text methods classify `builtin`. The field-type→kind
  mapping (blob/media/enum/option/text/code) is now a single shared `field_receiver_kind` helper
  used by BOTH the declared-record (`compound_field_receiver_kind`, renamed from
  `compound_blob_media_field_kind`) and implicit-Rec (`implicit_rec_field_builtin_kind`) paths,
  so they can no longer drift. CDO deps-loaded: compound-receiver 4→3, untracked-receiver 3→2,
  realUnknownRate 0.058% → 0.044%.
- **`Enum::"X"` static-type receivers.** `Enum::"CDO Module Type".Ordinals()` / `.Names()` —
  a static enum TYPE reference via the generic `Enum::` qualifier — now types as `Framework{Enum}`
  so its static methods classify `builtin` via the EnumType catalog (and `Ordinals`/`Names` chain
  to List). Only the literal `Enum::` form matches; a value reference `SomeEnum::Value` is left
  untouched. CDO deps-loaded: compound-receiver 6→4, realUnknownRate 0.073% → 0.058%.
- **`System` pseudo-singleton receiver.** `System.GetCollectedErrors()`, `System.Today()`, and
  the other qualified forms of AL's global runtime functions now classify `builtin` via a new
  `System` framework singleton (75-method catalog from the compiler `System` surface), wired
  into the Step-2c singleton interception alongside `Session`/`Database`/`NavApp`. CDO
  deps-loaded: untracked-receiver 5→3, realUnknownRate 0.088% → 0.073%.
- **`Text`/`Code`/`Label` static receivers + `this.<member>` self-qualifier.** Two Phase-A
  receiver-typing additions: (1) the static-type-receiver interception (previously Xml-only) now
  also covers `Text`/`Code`/`Label`, so `Text.CopyStr(...)` and the other Text data-type static
  methods classify `builtin` via the Text catalog when no variable shadows the bare type name;
  (2) a `this.<member>` receiver (the AL self-instance qualifier) strips the `this.` prefix and
  re-infers on the remainder, so `this.DialogWindow.Open()` resolves via the `DialogWindow`
  object global (Dialog). CDO deps-loaded: compound-receiver 8→6, untracked-receiver 9→5,
  realUnknownRate 0.131% → 0.088%.
- **`ControlAddIn`-typed variables resolve as control-add-in receivers.** A variable or
  parameter declared `ControlAddIn "X"` (e.g. `HTMLEditor: ControlAddIn "CDO.Editor"`,
  `editorAddIn: ControlAddIn "CDO.Editor"`) now classifies as the `ControlAddIn` framework
  receiver, so its member calls (`HTMLEditor.InitEditor(...)`, page-callback methods) classify
  `builtin` — JS-side platform invocations with no in-AL target — instead of
  `Unknown{NonObjectReceiverType}`. Same honest classification already applied to page
  UserControl receivers. CDO deps-loaded: non-object-receiver-type 6→0, realUnknownRate
  0.175% → 0.131%.

### Fixed
- **Quoted identifiers containing `(`/`[`/`.` parse as simple receiver names.**
  `simple_receiver_name` rejected any quoted identifier whose inner text contained `(` or `[`,
  misclassifying common BC field/var receivers like `"Request Page (xml)"`, `"Amount (LCY)"`,
  `"A.B"` as compound `call-result` expressions — so `"Request Page (xml)".CreateOutStream(...)`
  and friends fell to `Unknown{CompoundReceiver}`. Those characters are LEGAL inside an AL quoted
  identifier; only an embedded `"` (e.g. `"A"."B"`) signals a real compound. Now resolves the
  member call on the quoted field (Blob/stream intrinsics, etc.). CDO deps-loaded:
  compound-receiver 17→8, realUnknownRate 0.241% → 0.175%.
- **Compound framework chains accept RecordRef/FieldRef/KeyRef bases.** The single-hop
  framework-chain resolver (`compound_framework_property_kind`) only matched a
  `Framework{kind}` base, so `RecRef.Field(n).SetRange(...)` and `SourceRecRef.KeyIndex(1).M()`
  — whose base `RecRef` infers to the DEDICATED `ReceiverType::RecordRef` variant, not
  `Framework{RecordRef}` — fell to `Unknown{CompoundReceiver}`. A new `framework_kind_of` helper
  maps the dedicated `RecordRef`/`FieldRef`/`KeyRef` receiver-type variants to their catalog
  kind, so the chain resolves (`RecRef.Field(n)` → FieldRef → `SetRange`/`SetFilter` builtin).
  CDO deps-loaded: compound-receiver 22→17, realUnknownRate 0.278% → 0.241%.

### Added
- **Enum/Option table fields resolve as enum-value receivers.** An Enum/Option-typed table
  FIELD used as a member receiver — `Rec."eSeal Service".Ordinals()`,
  `EMailTemplateLine."Mail Importance".AsInteger()`,
  `EMailTemplateHeader."Report Selection Usage".AsInteger()` — now types as the new
  `Framework{Enum}` value-instance receiver (catalog `AsInteger`/`FromInteger`/`Names`/`Ordinals`
  from the compiler `EnumType` surface). The field-of-record compound resolver, previously
  blob/media-only, now recognizes enum/option fields via first-token data-type matching (covers
  native `Enum "X"` and dep-ABI `format_type` output). `framework_method_return_type` maps Enum
  `Names`/`Ordinals` → List, so the chained `Rec."eSeal Service".Ordinals().Count()` resolves.
  CDO deps-loaded: compound-receiver 31→22, realUnknownRate 0.343% → 0.278%.
- **Xml framework type names resolve as static receivers.** `XmlElement.Create(...)`,
  `XmlDocument.ReadFrom(...)`, `XmlDeclaration.Create(...)`, `XmlText.Create(...)` invoke STATIC
  factory/utility methods on the framework type itself. When the bare type name has no declared
  variable shadowing it, Phase A now types it as `Framework{Xml}` (an explicit allow-list of Xml
  value types — EXCLUDES `XmlPort`, an AL object type), so Phase B classifies the static method
  via the shared Xml builtin catalog. `framework_method_return_type` also maps the Xml `Create*`
  factories → Xml, so chained `XmlElement.Create(Name).AsXmlNode()` resolves. CDO deps-loaded:
  untracked-receiver 17→9, compound-receiver 35→31, realUnknownRate 0.431% → 0.343%.
- **Named return values are tracked as in-scope variables.** A procedure with a NAMED return
  value — `procedure CreateDefaulteDocsSendCode() SendCode: Record "CDO Send Code"` — exposes
  that name as a usable variable inside the body (`SendCode.Insert()`, `SendCode.GetX()`). The
  routine scope projection now seeds the named return as a record variable (when record-typed)
  AND a general scalar variable (any type: `Codeunit`/`Interface`/framework), mirroring a local
  declaration. Member calls on a named return now resolve instead of falling to
  `Unknown{UntrackedReceiver}`. CDO deps-loaded: untracked-receiver 28→17, realUnknownRate
  0.511% → 0.431%.
- **`ALDUMP_DEBUG_UNKNOWN` diagnostic** — `--l3-unknown-breakdown-cross-app` now honors the
  `ALDUMP_DEBUG_UNKNOWN` env var (set to `1` for all, or a substring to filter by receiver
  shape) to dump each residual unknown edge's owning object/routine + receiver shape + method
  to stderr. The work-list tool for locating the exact source behind each breakdown bucket.
- **Report dataitem names resolve as record variables.** AL lets you reference a report
  `dataitem(Name; "Source Table")` BY NAME as a record typed to its source table — e.g.
  `"Sales Header Filter".GetView()` / `.GetFilters()` / `.SetRange(...)` for
  `dataitem("Sales Header Filter"; "Sales Header")`. The dataitem name is in scope across ALL
  of the report's routines (report-level procedures + sibling dataitem triggers), so the routine
  projection now seeds EVERY dataitem's name as a record variable typed to its source table
  (`record_types` pass-1 resolves the `table_id` by name). Distinct from the per-dataitem
  implicit `Rec` of a dataitem trigger. Member calls on dataitem-named records now classify
  `builtin` instead of `Unknown{UntrackedReceiver}`. CDO deps-loaded: untracked-receiver 57→28,
  realUnknownRate 0.723% → 0.511%.

### Changed
- **Codeunit `TableNo` seeds an implicit `Rec`.** A codeunit with a `TableNo` property runs
  against an implicit `Rec` of that table (its `OnRun(var Rec)` parameter; `Rec` is exposed
  unqualified inside the codeunit), so `Rec.<proc>()` / `Rec.<field>` in such a codeunit now
  resolve instead of falling to `Unknown{UntrackedReceiver}`. `TableNo` is read in the routine
  projection (NAME or NUMBER) and set as the seeded `Rec`'s `table_name`; `record_types` pass-1
  now resolves either form via `resolve_table_ref_to_id`. CDO untracked-receiver 81→57,
  realUnknownRate 0.898% → 0.723%; DC untracked 153→85, 1.71% → 1.49% (DC has many TableNo
  processing codeunits).

### Added
- **Framework method/property return chains** — extends the single-hop framework-property
  compound resolver to framework METHOD calls that return a framework type:
  `JsonToken.AsValue()` → JsonValue, `XmlNode.AsXmlElement()` → Xml, `RecordRef.Field(n)` →
  FieldRef, `ErrorInfo.CustomDimensions` → Dictionary, etc. So a chain like
  `JTok.AsValue().AsInteger()` / `RecRef.Field(n).Value()` classifies `builtin` instead of
  `Unknown{CompoundReceiver}`. New `framework_method_return_type` map; `compound_framework_property_kind`
  now handles both the property and method-call form of `<prop>`. These AL framework conversions
  are deterministic (the return type never varies), so resolution is precise. CDO deps-loaded:
  compound-receiver 53→35, realUnknownRate 1.03% → 0.898%.
- **Single-hop call-result compound receivers** (Feature C2, engine-d22). A
  compound receiver `Func().Method(...)` — a member call ON THE RESULT of a bare
  own-object procedure with a KNOWN return type — now types the receiver as that
  return type and dispatches the method on it, instead of degrading to a
  `compound-receiver::call-result` unknown. `compound_call_result_receiver` in
  `receiver_type.rs` parses the bare `<Name>` (text before the first `(`, declining
  any `.`-bearing / non-bare form), resolves it to EXACTLY ONE same-name routine in
  the caller's object (mirroring `infer_call_expr_return_type`'s single-match
  precision gate; overloaded / absent / global-only names decline), reads its
  `return_type`, and classifies it via `parse_object_type_ref` (Object kinds) /
  `classify_receiver` (Record / framework kinds). PRECISION-FIRST: it DECLINES on
  ANY uncertainty — no return type, an Interface/Enum return, a primitive scalar /
  `Variant` / unparseable return — so a wrong return-type guess never masks a real
  hole. Example win: `HelperRec(Customer).FindSet()` (where `HelperRec(): Record
  Customer`) now classifies the `FindSet` as a Record `builtin`.
- **Single-hop framework-property compound receivers** (Feature C1, engine-d22).
  A compound receiver `<fw>.<prop>.<method>()` where the base types as a
  `Framework{kind}` and `<prop>` is a framework-returning property of that kind
  (e.g. `HttpClient.DefaultRequestHeaders.Add('k','v')`,
  `HttpResponseMessage.Content.ReadAs(...)`) now resolves to the property's
  framework type and classifies the method via the builtin catalog instead of
  degrading to a `CompoundReceiver` unknown. New `framework_property_type(kind,
  property_lc)` in `member_builtins.rs` maps the well-known Http* property returns
  (`HttpClient.DefaultRequestHeaders : HttpHeaders`, `Http{Request,Response}Message.{Content,Headers}`,
  `HttpContent.Headers`); `compound_framework_property_kind` in `receiver_type.rs`
  wires it as a single-hop compound resolver alongside the existing blob/media and
  CurrPage-control compound paths.
- **AL platform-type builtin catalogs — non-object-receiver win** (Feature A,
  engine-d22). The `non-object-receiver-type` unknown bucket previously included
  member calls on AL platform value types (`Notification`, `ErrorInfo`, `Text`,
  `RecordId`, etc.) that have real builtin method surfaces but were not wired into
  the resolver's builtin catalog. 26 new `ReceiverBuiltinKind` variants + `phf_set!`
  catalogs (method counts: Notification 9, ErrorInfo 18, ModuleInfo 7, RecordId 2,
  BigText 6, SecretText 3, DataTransfer 9, SessionSettings 9, Text/Code/Label 32,
  Date 6, DateTime 3, Time 5, Guid 3, Integer 1, Decimal 1, Boolean 1, Duration 1,
  BigInteger 1, Byte 1, File 28, FileUpload 2, NumberSequence 7, Version 6,
  FilterPageBuilder 11, SessionInformation 4). `classify_receiver` now also strips
  length suffixes (`Text[1024]` → `text`, `Code[20]` → `code`). `Code` and `Label`
  alias to the `Text` kind. Sourced from `tools/gen-al-builtins/out/member_builtins.json`.

### Changed
- **L3 analysis scopes to one app at nested-`app.json` boundaries** (multi-app / monorepo
  support). The disk assembly (`assemble_l3_workspace_from_disk`, used by `aldump` + the
  cross-app stats) previously fail-closed when a workspace contained more than one `app.json`
  anywhere in its tree — so a monorepo with a root app plus nested sub-apps (e.g. Continia
  Document Capture: root + `Modules/Purchase Contracts/{Base,Integration}`) could not be
  analyzed at all. New `discover_al_files_app_scoped` treats a child directory carrying its
  own `app.json` as a SEPARATE project (the AL compiler's own semantics) and does NOT descend
  into it, so the targeted app's source is analyzed in isolation; each nested app is analyzed
  by pointing the workspace at its own root. The `count_app_json > 1` guard is dropped from
  this path (a missing/id-less root `app.json` still fail-closes via `read_root_app_guid`).
  The GATE keeps its own stricter multi-app provider check (`workspace_diagnostics`) — only
  the analysis path is relaxed. Unblocks Document Capture (28.4k edges, source-only
  realUnknownRate 1.83%) and its module apps.

### Fixed
- **Quoted scalar variable names strip their quotes** (consistency with parameter and
  record-variable extraction). `extract_variables` (locals) and `extract_object_globals` keyed
  a `quoted_identifier` variable by its raw text INCLUDING quotes (`"file blob"`), but
  `simple_receiver_name` returns the inner unquoted name (`file blob`), so a member call on a
  quoted scalar variable `"My Var".M()` missed the variable lookup → `Unknown{UntrackedReceiver}`.
  New `decl_name_lc` helper strips quotes on both scalar sites, matching the param/record-var
  treatment. (No metric change on CDO — its residual untracked names are Blob FIELDS, not
  quoted variables — but removes the latent asymmetry.)
- **Grouped multi-name variable declarations capture every name.** The AL grammar's
  `variable_declaration` multi-name arm (`A, B, C : Type;`) emits one `name` field per
  variable, but `scope.rs` read only `child_by_field_name("name")` (the FIRST), silently
  dropping `B`/`C` across all four extraction sites (local vars, object globals, local record
  vars, object-global record vars). Trailing names in a group were therefore untyped →
  `Unknown{UntrackedReceiver}` on any member call (and invisible to L5 detectors). New
  `decl_name_nodes` helper iterates `children_by_field_name("name", …)`; each declared name
  becomes its own symbol. CDO deps-loaded: untracked-receiver 147→136, realUnknownRate
  4.4941% → 4.4182%. No fixture uses grouped decls, so all goldens stay byte-stable.
- **Dependency symbols: recurse `Namespaces[]`** — the single biggest cross-app resolution
  hole. `engine::deps::symbol_reference::parse_symbol_reference` read only TOP-LEVEL object
  arrays (`Pages`, `Codeunits`, `Tables`, …). BC 24+ apps (every modern Microsoft + ISV
  `.app`) nest objects under `Namespaces[]` nodes, so the parser dropped almost the entire
  dependency object/routine/table set (Microsoft Base Application 28.0: top-level Pages = 10,
  recursive = 2609 — ~99% lost). `raw_objects` now recurses every `Namespaces[]` level via
  `collect_raw_objects`. Combined with the three resolution fixes below, drove CDO
  deps-loaded realUnknownRate **6.6767% → 4.4941%** (unknown 933→628, resolved 7390→7952,
  external 304→15, record-table-procedure 296→0). Flat (pre-BC24) `.app`s are unaffected
  (no `Namespaces` node → no recursion), so all existing goldens stay byte-stable.

### Changed
- **Member-of-member Blob/Media field receivers resolve.** A compound receiver
  `<recvar>.<field>` where `field` is a `Blob`/`Media`/`MediaSet` field of the record's table
  (`DOTempBlob.Blob.CreateOutStream(...)`, `PDFDocument."File Blob".CreateInStream(...)`) now
  classifies the field intrinsic as `builtin` instead of `Unknown{CompoundReceiver}`.
  `infer_receiver_type` splits on the LAST `.`, resolves the base record's table, and looks up
  the field — reusing the Blob/Media catalogs. Deeper chains (`CurrPage.<Part>.Page`) still
  decline (the base is itself compound). CDO deps-loaded: compound-receiver 243→170,
  realUnknownRate 2.88% → 2.34%.
- **Table procedures (not just triggers) seed the implicit `Rec`.** `implicit_base_receiver`
  only registered the implicit current record for table/tableextension TRIGGERS, but AL exposes
  the table's fields and procedures unqualified inside ANY of its methods. Broadened to table
  procedures, so (a) bare record-builtin calls (`Modify()`, `SetRange()`, …) in a table
  procedure are correctly captured as RECORD OPERATIONS on `Rec` instead of phantom
  global-builtin call edges; (b) explicit `Rec.<proc>()` and bare field accesses resolve. CDO
  deps-loaded: untracked-receiver 136→81, realUnknownRate 3.208% → 2.88% (266 phantom builtin
  call edges reclassified to record operations — a more accurate call graph, not lost edges).
  Regenerated `ws-d40` r1a/r2a goldens (the one fixture with a table procedure) — adds its
  implicit `Rec` record variable; no call-graph/coverage/detector golden changed.
- **Blob / Media field receivers resolve to field intrinsics.** A `Blob`/`Media`/`MediaSet`
  table FIELD used as a member receiver — bare on the implicit `Rec` (`"File Blob".CreateInStream(...)`)
  or as a declared `Blob` variable — now classifies the field intrinsic
  (`CreateInStream`/`CreateOutStream`/`HasValue`/`Length`; media import/export/query) as
  `builtin`. New `ReceiverBuiltinKind::Blob`/`Media` + catalogs; `classify_receiver` maps the
  type names; `infer_receiver_type` resolves a bare blob/media field of the implicit Rec's
  table.
- **Bare calls resolve against the implicit `Rec` (SourceTable) procedures.** AL treats an
  unqualified call in page/table code as `Rec.<proc>()`, so a bare call to a SourceTable
  procedure is legal (e.g. `GetTemplateVariantCaption()` in a page bound to the table that
  defines it; `Navigate()` resolving to the base table's `Navigate`). `PCallee::Bare` now adds
  a fallback (after own-object and extends-target, before global-builtin/`BareUnresolved`):
  resolve the caller object's implicit table (Table self / Page `SourceTable` / extension
  base) ∪ its TableExtensions via `resolve_by_name_and_arity_multi`. Own-object procedures are
  still tried FIRST so they shadow a same-named table procedure. New
  `implicit_rec_table_object_id` helper (NAME- or NUMBER-form table ref). CDO deps-loaded:
  bare-unresolved 169→0, realUnknownRate 4.4182% → 3.208% (resolved +170). The fallback only
  binds to a REAL name+arity match, so it cannot invent edges.
- **Record member dispatch searches base table ∪ its TableExtensions.** A `TableExtension`
  procedure is globally callable on the base record in AL but lives under the extension's own
  object id, so `routines_in_object(base_table)` missed it (false `Unknown{RecordTableProcedure}`).
  Added `SymbolTable::table_extension_object_ids` (TableExtensions indexed by extends-target
  name AND number) + `resolve_by_name_and_arity_multi` (one candidate pool over a set of
  object ids); `dispatch_record` now unions the base table with every TableExtension extending
  it. Resolves e.g. CDO's `Rec.CDOOpenEmail()` (defined in a CDO `TableExtension` on a base
  BC table).
- **Numeric `SourceTable` / extends-target resolution.** Dependency `.app` symbols encode a
  page's `SourceTable` and an extension's extends target as the table's object NUMBER (e.g.
  `"5992"`); native AL source uses the table NAME. `record_types::resolve_table_ref_to_id`
  resolves both forms — a numeric ref routes through `object_by_type_number("Table", n)`
  (type-qualified) → name → `L3Table.id`. Lets a PageExtension's implicit `Rec` bind to its
  base page's SourceTable when that base page is a dependency object.
- L3 implicit-`Rec`/`xRec` receiver typing: a member call on the implicit record now types as
  `ReceiverType::Record` whenever a `record_variables` entry exists for it, REGARDLESS of
  whether its table object id resolves (a cross-app SourceTable leaves `table_id` None). Phase
  B then decides honestly (builtin → `builtin`; table procedure on an unresolved table →
  `RecordTableProcedure`). Mirrors the existing table-id-independent decision for declared
  record vars. Diagnostic: `RecordTableProcedure` edges now carry a `receiver_shape` sub-cause
  tag (`table-unresolved::…` vs `proc-not-found::…`) for `--l3-unknown-breakdown[-cross-app]`.

### Added
- **Page-control resolution — `CurrPage.<control>…` member calls.** New `L3Object.page_controls`
  (`L3PageControl { name, kind: Part/SystemPart/UserControl, target }`), populated from BOTH the
  native AL layout (tree-sitter `part_section`/`systempart_section`/`usercontrol_section`) and
  dependency `.app` symbols (`Controls[]` integer `Kind`: 6=Part → subpage page NUMBER via
  `RelatedPagePartId.Id`, 10=UserControl → add-in name via `RelatedControlAddIn`; recursed through
  nested controls). `SymbolTable::page_controls_for(object_id)` merges a PageExtension's own
  controls with its base page's. At resolution, `currpage_control_receiver` (a "Step 0" in
  `infer_receiver_type`) resolves:
  - `CurrPage.<Part>.Page.<m>()` / `CurrPage.<Part>.<m>()` → the subpage **Page object's** procedure
    (subpage found by NAME in native source, NUMBER in dep symbols; Phase B dispatches the Page
    receiver's method by name+arity — object-run is Codeunit-gated, so this is a plain procedure
    lookup).
  - `CurrPage.<UserControl>.<m>()` → a control-add-in `builtin` edge (below).
  CDO deps-loaded: compound-receiver 170→62, realUnknownRate **2.336% → 1.548%** (resolved +63,
  builtin +37; total edges unchanged). No fixture exercises page controls, so all goldens stay
  byte-stable.
- **`CurrPage.<UserControl>.<method>()` resolves to a control-add-in `builtin` edge.**
  A page `usercontrol(Body; "Some AddIn")` accessed as `CurrPage.Body.SetContent(...)`
  is a platform/JS-side control-add-in invocation with no in-AL target. Phase A's
  `currpage_control_receiver` now types a `UserControl` control as the new
  `ReceiverBuiltinKind::ControlAddIn` framework receiver; Phase B's `dispatch_framework`
  classifies EVERY method on it as `builtin` (we cannot enumerate an add-in's JS method
  surface, and these are genuine platform calls — never real-`unknown`, and not the
  runtime-typed `dynamic` dispatch). Previously these declined to
  `Unknown { CompoundReceiver }`. Test in `tests/l3cg_page_part_dispatch.rs`.
- **Extension bare-call resolver**: when a bare call in a `PageExtension` /
  `TableExtension` / `ReportExtension` / `EnumExtension` is not found in the caller's own
  object, the resolver now falls back to the EXTENDS-TARGET base object's procedures before
  emitting `Unknown{BareUnresolved}`. Order: own-object → extends-target base → global
  builtin → `BareUnresolved`. Adds `SymbolTable::object_by_id` (exact-id index) and
  `extends_base_object` helper in `call_resolver.rs`. CDO cross-app (deps-loaded): unknown
  943 → 933 (−10 bare-unresolved edges now resolved); source-only: unchanged (CDO base
  pages are dep objects, only visible when `.alpackages` are loaded).
- `aldump --l3-unknown-breakdown-cross-app <workspace>`: the DEPS-LOADED, PRIMARY-scoped
  unknown breakdown — the north-star work-list. Same merged-model + primary-edge scoping as
  `--l3-call-graph-stats-cross-app`, but attributes every residual TRUE-`unknown` edge to its
  `UnknownReason` (`byReason` / `receiverShapeDetail` / `bareCallDetail` /
  `frameworkMethodDetail`) so the real whole-program holes can be targeted directly rather
  than inferred from the source-only breakdown. Fail-closed → message + empty breakdown.
- `aldump --l3-call-graph-stats-cross-app <workspace>`: deps-loaded, PRIMARY-scoped
  honest-taxonomy histogram. Builds the cross-app merged model (workspace `.al` source +
  dep `.app`s under `.alpackages`), runs call resolution with the real declared/fetched dep
  ledger, then scopes the histogram to **primary (workspace) edges only** — edges whose
  `from` routine is NOT a dep routine (`dep_routine_ids = {r | r.app_guid ∈
  fetched_app_guids}`). Same JSON shape as `--l3-call-graph-stats` plus `depAppsLoaded`.
  This is the honest whole-program real-`unknown` rate (dep symbols present for resolution;
  dep-internal call sites excluded from the denominator). CDO baseline (10 dep apps loaded):
  source-only 6.88% → deps-loaded primary 6.75% (resolved 7120→7380 +260; unknown 961→943
  -18; external reclassified from unknown 558→304 with cross-app resolution active).

### Changed
- L3 member dispatch: a `Variant`-typed receiver now classifies `dynamic` (spec §6
  honest taxonomy — the held type is runtime-determined) instead of real-`unknown`.
  `ReceiverType::Dynamic` + `dynamic_method` emit a `dispatch_kind = Dynamic` edge. CDO:
  non-object-receiver-type 70→68, realUnknownRate 6.89%→6.88% (no new resolved edges).

### Fixed
- **Witness reachability via reverse-BFS valid-node set** in `reconstruct_witness_paths`
  (Case C inherited-fact BFS): the per-edge `can_reach` memoized check (which scanned
  the full direct-∪-inherited capability cone per node, calling `fact_equivalent` ~750k
  times per root on the CDO app) is replaced by a **one-shot reverse-BFS** computed once
  per `reconstruct_witness_paths` call. Carrier nodes (those with a direct fact equivalent
  to the target) are found by scanning `direct_facts_by_routine` (far fewer facts than the
  inherited cone). A reverse-BFS from those carriers over the new `incoming_edges` index
  (reverse of `typed_edges`, built once in `build_fingerprint_indexes`) computes
  `valid_nodes: HashSet<&str>` — the set of nodes that can reach `fact` in the forward
  call graph. The per-edge prune is now an O(1) `valid_nodes.contains(to)` check.
  Correctness: `facts_by_routine[N].any(equiv fact)` ≡ "N is an ancestor-of-or-equal-to
  some carrier in the forward graph" ≡ "N ∈ reverse-BFS from carriers" — the valid set is
  identical. All goldens and contracts remain byte-stable. CDO `alsem analyze` wall time
  ~20 min → < 1 min.
- **Skip non-ordering witness reconstruction** in `compute_digest_effects_for_ordering`:
  the ordering engine only grades `DB_INSERT / DB_MODIFY / DB_DELETE / COMMIT / HTTP /
  FILE / UI_CONFIRM / UI_MESSAGE / UI_WINDOW_OPEN / ERROR_THROW`; for all other effect
  types it treats effects with empty `via_paths` and `owner == routine_id` as direct
  (empty `CallChain`). The new `ordering_witness_only: bool` parameter to `digest_query`
  (passed `true` from `compute_digest_effects_for_ordering`, `false` from all other paths)
  skips `reconstruct_witness_paths` for non-ordering-relevant effect types, emitting the
  effect with empty `via_paths`. Digest shape and `scoped_guarantees` are unchanged; the
  R4-F and CLI-B goldens remain byte-stable.
- **Parent-pointer arena BFS** in `reconstruct_witness_paths` (Case C inherited-fact
  witness): replaced the cloned `State { routine, hops: Vec<WitnessHop>, visited:
  HashSet<String> }` (cloned in full on every edge expansion) with a `Node { routine,
  hop, parent, depth }` arena + `VecDeque<usize>` index queue. Visited-set check is now
  O(depth) via a `Vec<String>` parent-chain walk (one allocation per *popped* node, shared
  across all out-edge checks for that node). Path materialisation walks parents on
  completion only (rare). Eliminates the `O(depth * out_degree)` per-expansion clone of
  both the `HashSet<String>` and the `Vec<WitnessHop>` that dominated the per-state cost
  (~46 µs/state). Eliminates per-expansion allocation overhead; all existing goldens and
  contracts remain byte-identical. (CDO `analyze` wall time is dominated by the total
  number of `(root, fact)` BFS invocations on large workspaces, which this change does not
  address — see next milestone.)
- L5 ordering/digest witness reconstruction no longer blows up on dense call graphs
  (the Record-table-procedure + implicit-Rec dispatch edges densified out-degree, which
  made `alsem analyze` effectively non-terminating on the CDO app — 15k+ CPU-s). Three
  behavior-preserving fixes (all `*.l3*`/r4f/digest/cli-b goldens byte-stable): (1)
  **reachability-directed pruning** in `reconstruct_witness_paths` — a frontier edge whose
  target cannot reach the target fact (per the already-computed `facts_by_routine` cone)
  is skipped, discarding the dead-end subtrees that exhausted the 25k-state budget (was
  ~83% of calls hitting the cap → 0%); (2) out-edges **pre-sorted once** at index build
  instead of cloned+sorted per BFS state; (3) `compute_ordering_facts` restricted to roots
  whose cone carries an IO/UI effect (the only roots that can yield an ordering label),
  via the new `compute_digest_effects_for_ordering` — skipped roots produce empty ordering
  facts, so the result is identical.

### Added
- **AL singleton-type static receivers → builtins** (`src/engine/l3/member_builtins.rs`,
  `src/engine/l3/receiver_type.rs`): `infer_receiver_type` Step 2c now intercepts the
  AL platform singleton type names (`IsolatedStorage`, `Session`, `NavApp`,
  `TaskScheduler`, `Database`, `Page`, `Report`) in addition to the existing
  `CurrPage`/`CurrReport` intercepts, before emitting `UntrackedReceiver`. Five new
  `ReceiverBuiltinKind` variants are added (`IsolatedStorage` 5 methods,
  `Session` 19, `NavApp` 16, `TaskScheduler` 5, `Database` 29); `Page`/`Report` bare-name
  singletons reuse the existing `PageInstance`/`ReportInstance` catalogs. Phase B's
  existing `Framework` arm dispatches via the catalogs: catalog hit → `builtin`,
  catalog miss → `Unknown { FrameworkMethodNotInCatalog }` (honest gap). The
  variables-first check (Step 2) is preserved — a user variable named `Session` correctly
  shadows the singleton. 6 new tests in `tests/l3cg_singleton_static_dispatch.rs`.
  CDO `DocumentOutput/Cloud` (13,971 total edges): `unknown` 1,093 → 963 (−130),
  `builtin` 5,079 → 5,209 (+130), `resolved` UNCHANGED at 7,120 (pure reclassification);
  `realUnknownRate` 7.82% → 6.89% (−0.93 pp). Breakdown: `page` −50, `isolatedstorage`
  −38, `report` −16, `session` −13, `navapp` −9, `taskscheduler` −4.
- **Name residual unknowns in `--l3-unknown-breakdown`** (`src/engine/l3/call_resolver.rs`,
  `src/engine/l3/receiver_type.rs`, `src/engine/l3/resolution_class.rs`, `src/bin/aldump.rs`):
  the `BareUnresolved` path now threads the lowercased call name onto `CallEdge::unknown_method_name`
  so the breakdown can emit a per-name count histogram (`bareCallDetail`). Untracked-receiver
  `other` shapes now embed the actual variable name in the shape tag
  (`"other::<name>"` instead of a flat `"other"`) and compound-receiver `member-of-member`
  shapes embed the receiver expression (truncated to 120 chars), so `receiverShapeDetail`
  surfaces concrete identifiers. `unknown_breakdown` returns a 4-tuple (adding `bareCallDetail`
  split from the framework-method detail); `aldump` emits the new field. **Purely diagnostic —
  zero resolution/classification changes, zero golden changes.** On CDO (13,971 edges, 1,093
  true unknowns): 188 `bare-unresolved` names are now named; all 188 are user-defined
  application procedures (none are genuine platform globals — confirmed against the AL 18.0
  compiler DLL's ClassDocumentationResources); the untracked-receiver `other` bucket (252
  edges) now shows concrete names including `IsolatedStorage` (38), `Page` (50), `Report`
  (16), `Session` (13), `NavApp` (9), `TaskScheduler` (4) — a road-map for future typed-
  receiver static-method resolution.

- **Task 6a — Implicit Rec/xRec receiver resolution** (`src/engine/l3/receiver_type.rs`):
  `infer_receiver_type` Step 2b now checks `routine.record_variables` BEFORE yielding
  `UntrackedReceiver`. For Table/Page/TableExtension/PageExtension objects, pass 3 of
  `record_types::resolve_routine_record_types` sets `table_id` on the implicit `Rec`/`xRec`
  record variable. Step 2b finds this entry (case-insensitive name match, `table_id == Some`),
  walks it through `symbols.table_by_id` → `symbols.object_by_type_name("Table", name)`, and
  returns `ReceiverType::Record { table_object_id: Some(..) }` so Phase B can dispatch both
  catalog builtins (`TableCaption`, `FieldNo`, etc.) and real user table procedures. A codeunit
  with an undeclared `Rec` (no effective own table → `table_id == None`) stays
  `Unknown { UntrackedReceiver }` (correct: no false resolution). The previously deferred
  `implicit_rec_table_procedure_deferred` test in `tests/l3cg_record_dispatch.rs` has been
  promoted from "stays unknown" to "now resolves". Four new tests in
  `tests/l3cg_implicit_rec_dispatch.rs` cover: table trigger resolves, builtin stays builtin,
  page-via-SourceTable resolves, and codeunit stray Rec stays unknown.
- **Task 6a — Receiver-shape sub-characterization in `--l3-unknown-breakdown`**:
  Added `receiver_shape: Option<String>` field to `CallEdge` (DIAGNOSTIC-only, never projected
  to golden output). `InferredReceiver` now carries `receiver_shape: Option<String>` set by
  Phase A helpers: `compound_receiver_shape` (classifies `member-of-member` / `call-result` /
  `indexed` / `other`) for `CompoundReceiver` edges, and `untracked_receiver_shape` (classifies
  `implicit-rec` / `currpage` / `currreport` / `other`) for `UntrackedReceiver` edges. Phase B's
  `Unknown` arm propagates the shape onto the emitted edge. `resolution_class::unknown_breakdown`
  now returns a 3-tuple adding `receiverShapeDetail` (keyed by `"{reason}::{shape}"`), and
  `aldump --l3-unknown-breakdown` exposes this as `"receiverShapeDetail"` in the JSON output.
- **Phase 3 — Record table-procedure dispatch** (`src/engine/l3/call_resolver.rs`): member
  calls on `Record <Table>`-typed variables where the method is NOT a built-in intrinsic are
  now resolved to the table's user-defined procedure. The resolver looks up the receiver's
  table object id via `routine.record_variables` (resolved by `record_types` pass 1/3) then
  falls back to parsing the declared type via `record_types::record_table_name_of`, then calls
  `resolve_by_name_and_arity` with full arity/overload disambiguation. Edges become
  `resolution=resolved`, `dispatchKind=method`, `to=<routine-id>`. CDO `DocumentOutput/Cloud`
  impact: `record-table-procedure` unknown edges 806 → 66 (−740), `resolved` 6358 → 7098
  (+740), `realUnknownRate` 15.68% → 10.39% (−5.29 pp). Residual 66 unknowns are genuine
  non-resolvable cases: implicit `Rec` in table triggers (deferred to Task 6 — the implicit
  `Rec` is NOT in `routine.variables` so Step 2 returns UntrackedReceiver before Phase 3
  fires), plus calls on record vars from unindexed external tables. Detector delta vs 1867
  baseline: PENDING (analysis in progress; no new golden failures; oracle invariants pass).
  Contract oracle (Invariant 2: every resolved `to` exists in the symbol table) verified.
  Deferred: implicit-Rec table-trigger dispatch (requires Task 6 ReceiverType lattice).
  New tests in `tests/l3cg_record_dispatch.rs` (5 tests: resolve, builtin-unchanged,
  missing-stays-unknown, implicit-rec-deferred, arity-overload).
- L3 call-graph contract oracle (`tests/l3cg_oracles.rs` Invariant 11): a bare call to an
  AL platform GLOBAL function (Task 2 catalog) classifies `builtin` on the BARE path
  (dispatchKind "builtin"), is disjoint from `resolved` (no edge is both builtin and
  resolved), and a genuine non-global bare miss STILL classifies `unknown` (the catalog
  never swallows a real hole). Locks the clean-reclassification baseline before the
  graph-expansion phases. CDO `DocumentOutput/Cloud` cumulative after Tasks 1-3:
  `realUnknownRate` 23.6% → 15.68%, unknown 3295 → 2191, builtin 3639 → 4743, resolved
  unchanged at 6360 (pure reclassification, zero new resolved edges); `alsem analyze`
  1867 findings (detector baseline for the graph-expansion FP checks).
- Generated AL global-builtin catalog (`src/engine/l3/global_builtins.rs`): offline
  generator (`tools/gen-al-builtins/`) extracts all 785 distinct compiler-intrinsic method
  names from the AL compiler DLL's `ClassDocumentationResources` embedded resource
  (source: `Microsoft.Dynamics.Nav.CodeAnalysis.dll`, AL extension `ms-dynamics-smb.al-18.0.2293710`,
  97 types). The catalog is a `phf::phf_set!` checked into source; the generator is
  offline/manual (not in CI). Bare calls not resolved to the caller's own object whose
  name matches any catalog entry are reclassified from `unknown` (BareUnresolved) to
  `builtin` — a pure reclassification (no new resolved-to-routine edges). CDO impact on
  `DocumentOutput/Cloud`: bare-unresolved dropped 1247 → 188 (−1059), unknown total
  3295 → 2236, `realUnknownRate` 23.6% → 16.0%; resolved count unchanged at 6360.
- L3 call-graph: intrinsic built-in catalog (`src/engine/l3/member_builtins.rs`, `phf`
  perfect-hash) for Record / RecordRef / FieldRef / KeyRef + framework types (Json*,
  Http*, In/OutStream, TextBuilder, Dialog, List, Dictionary, Xml*). AL's
  compiler-intrinsic member methods (not present in any `.app` `SymbolReference.json`)
  now classify as `builtin` on the member resolution path instead of `unknown`. Phases
  1–2 of the call-graph resolution redesign (`docs/superpowers/specs/2026-06-13-call-graph-resolution-redesign.md`).
- Honest resolution taxonomy classifier (`src/engine/l3/resolution_class.rs`) +
  `aldump --l3-call-graph-stats` measurement harness reporting per-bucket edge counts
  and the real-`unknown` edge rate (the north-star metric).
- `aldump --l3-unknown-breakdown` + resolver-attributed `UnknownReason` on every
  `unknown` edge: attributes the residual real-`unknown` rate to its causes
  (bare-unresolved / record-table-procedure / untracked-receiver / compound-receiver
  / framework-method-not-in-catalog / non-object-receiver-type / enum-static /
  callee-unknown / interface-no-impl). The work-list for the typed-resolution phases.
  Measured on CDO (3295 unknown): bare-unresolved 1247, untracked-receiver 881,
  record-table-procedure 812, compound-receiver 243, non-object-receiver-type 70,
  framework-method-not-in-catalog 39, interface-no-impl 2, enum-static 1.
- `aldump --l3-unknown-breakdown` now includes `"frameworkMethodDetail"` in the JSON
  output: a per-`(KindName::method)` breakdown of `framework-method-not-in-catalog`
  edges, sourced from the new `CallEdge.unknown_method_name` diagnostic field. Helps
  identify specific catalog gaps without full call-graph inspection.
- Member-builtin catalog expanded from compiler JSON (`member_builtins.json`) closing
  all 18 `framework-method-not-in-catalog` unknown edges on the CDO workspace (from 39
  pre-global-builtin reclassification). Key additions: RecordRef `setrecfilter` + 26
  new Builtin entries; Record 14 new methods (arefieldsloaded, currentcompany,
  fullyqualifiedname, istemporary, readconsistency, readisolation, recordlevellocking,
  relation, securityfiltering, setascending, setbaseloadfields, tablename, truncate,
  loadfields); FieldRef 11 new enum-reflection methods; Json* types 35+ methods
  (GetArray/GetObject/GetText etc., SelectTokens, clone, YAML variants); Http*
  types expanded with certificate, cookie, secret-URI support; TextBuilder capacity
  methods; Dialog confirm/error/message/strmenu; XML types full union of all Xml*
  compiler types (60+ net-new entries). Pure reclassification — resolved count
  unchanged. CDO after: `framework-method-not-in-catalog` = 0, unknown 2209→2191,
  realUnknownRate 15.8%→15.7%.
- **CurrPage / CurrReport receiver resolution → Page / Report-instance builtins**
  (`src/engine/l3/member_builtins.rs`, `src/engine/l3/receiver_type.rs`): the two
  AL language singletons `CurrPage` and `CurrReport` — which are not declared variables
  but are the current page / report instance inside triggers — were classified as
  `Unknown { UntrackedReceiver }` with receiver-shape `currpage`/`currreport`. They
  are now intercepted in `infer_receiver_type` Step 2c (before `UntrackedReceiver` is
  emitted) and mapped to `ReceiverType::Framework { kind: PageInstance }` /
  `ReceiverType::Framework { kind: ReportInstance }`. Two new `ReceiverBuiltinKind`
  variants (`PageInstance` — 19 methods; `ReportInstance` — 36 methods) are added to
  the member-builtin catalog, sourced from `member_builtins.json` `"Page"` and
  `"ReportInstance"` arrays. Phase B's Framework arm dispatches via the catalog: a
  hit emits `builtin`; a miss emits `Unknown { FrameworkMethodNotInCatalog }` (an
  honest catalog gap, not a regression). Pure reclassification — `resolved` count
  unchanged. CDO `DocumentOutput/Cloud` after: `untracked-receiver::currpage` 319 → 0,
  `untracked-receiver::currreport` 15 → 0, builtin 4745 → 5079 (+334), unknown
  1427 → 1093 (−334), `realUnknownRate` 10.21% → 7.82% (−2.39 pp). Four new tests
  in `tests/l3cg_currpage_dispatch.rs`.

### Changed
- **Member-call resolution refactored to the ReceiverType lattice** (Phase A infer + Phase B
  dispatch) — `src/engine/l3/receiver_type.rs` (new) + `src/engine/l3/call_resolver.rs`. The
  deeply-nested string-keyed if/else ladder in `resolve_call_site`'s `PCallee::Member` arm
  (including the verbose surgical Record-table-procedure block) is replaced by a clean
  two-phase typed resolver: `infer_receiver_type(receiver, routine, symbols) -> ReceiverType`
  (a type lattice: Object / Interface / Enum / Record / RecordRef / FieldRef / KeyRef /
  Framework / Primitive / Unknown), then `dispatch(receiver_type, method, ctx) -> Vec<CallEdge>`
  (one match arm per variant). The surgical Record special-casing is ABSORBED into the Phase-B
  Record arm, preserving the catalog-builtin-FIRST ordering (a Record intrinsic like `SetRange`
  stays `builtin` even when the receiver's table is out-of-source). Strangler-Fig Phase A/B:
  wiring only — no new inference sources. Behavior-preserving (ZERO golden changes; CDO
  `DocumentOutput/Cloud` unchanged at resolved 7098 / builtin 4743 / unknown 1451 /
  realUnknownRate 10.39%). New direct unit tests on `infer_receiver_type` prove each lattice
  variant is inferred for a representative declared type.
- L3 taxonomy refactor: replaced the stringly-typed `CallEdge.dispatch_kind: String` /
  `resolution: String` (a TS-port hangover) with strict Rust enums `DispatchKind` /
  `Resolution` (`src/engine/l3/taxonomy.rs`). `Resolution::Unknown(UnknownReason)` folds
  the former `unknown_reason` side-field into the enum payload, so every `unknown` edge
  carries a compiler-enforced cause ("unattributed" is now structurally impossible);
  added `UnknownReason::DynamicObjectRunTarget` for the dynamic object-run edge.
  `enum.as_str()` reproduces the exact golden strings at the projection boundary — the
  refactor is internal-only and fully byte-stable (zero golden changes).
- L3 member-call resolution: a Record/framework receiver whose method is a recognized
  intrinsic now resolves to `builtin` (and leaves `unresolvedCallsites`). Non-intrinsic
  Record methods (real table procedures) remain `unknown`, pending Phase 3. Rebaselined
  the moved L3 call-graph + L3 coverage goldens (builtin reclassification only; no new
  resolved-to-routine edges) and updated the r2b `coverageMatrix.builtin` oracle
  (18→49). `KNOWN_DIVERGENCES.json` stays `[]`.
- **Test oracle: al-sem byte-parity RETIRED.** The engine is now Rust-owned; tests assert
  Rust-owned baselines + structural contracts, not equality vs the al-sem TS reference.
  The builtin reclassification correctly propagates downstream: r3a2 L4-summary phantom
  `unresolved-call` uncertainties removed (matrix 99→58); the `--require-dependencies`
  gate preflight reports coverage complete on builtin-only fixtures (exit 4→0, 28 rows;
  12 genuinely-degraded fixtures keep exit 4); and the `ws-txn-d48-pos` d48 finding's
  confidence rises `possible`→`likely` (a phantom `HttpClient.Send` uncertainty removed).
  See CLAUDE.md "Testing Philosophy & Goldens". Legacy al-sem-byte-parity tests
  (cli-b digest/fingerprint/prove/snapshot, r3a1, r4f_snapshot, gate_prsummary preflight
  oracles) are pending migration to Rust-owned baselines.

### Fixed
- Implicit-Rec argument bindings now flow `sourceTempState` (a pre-existing gap from the
  d22 implicit-Rec work): a trigger forwarding the implicit `Rec` to a record-mutating
  helper (`OnAfterInsert → Helper(Rec) → Rec.Modify()`) now resolves the cross-call
  inherited effect's temp-state to `Known(false)` instead of degrading to `Unknown`. The
  d22 work had rebaselined the d40 golden to expect `Known(false)` but never wired the
  temp-state through the binding, leaving r3a2/r4/gate red at the branch baseline.
- Rebaselined goldens after the iter-2 detector-gap fixes (G-13..G-19). Only **G-15**
  (d3 ignores field-writes/post-Init reads after a `Get`; d42 excludes PK-only fields)
  moved finding content; G-13/G-14/G-16/G-17/G-18/G-19 moved no in-repo goldens. The
  moves are all d3 suppressions/shrinks: (a) `ws-d8-commit-in-tx` — the d3 `rootCause`
  / `fixHint` field-set shrinks from `[last posting date, no., status posted]` to
  `[no.]` (the two written fields are excluded; the PK read `no.` survives), finding
  count unchanged; (b) `ws-txn-d46-pos` (if-not-`Get`-then-`Init`/`Insert` and
  `if Get then write` construct/upgrade patterns), `ws-txn-d47-pos-*` and
  `ws-txn-d49-pos-*` (write-after-`Get`: field `:= …; Modify()`), and
  `ws-rollup-multi-detector` (write-after-`FindSet`) — the d3 finding is now fully
  SUPPRESSED, dropping it from cli-a json/html/terminal/stats, gate SARIF/PR-summary,
  and the gate exit-code matrix (`--fail-on` info/low/medium for those default-slot
  fixtures now exits 0, not 1). The gate-suppress anti-degenerate witness
  (`ws-inline-suppress` `UnsuppressedD3`, which reads the Normal field `Name`) was
  CONFIRMED to survive G-15; its companion `SuppressedIo`/`WrongDirectiveIo` d3
  findings were write-after-`Get` and are now correctly suppressed, lowering the
  inline-suppress SARIF totals 7→5 (unsuppressed) and 6→4 (suppressed) while the d47
  suppression invariant (2→1) is unchanged. Extended the `REGEN_TEMP_GOLDENS` regen
  path to the cli-a stats and gate PR-summary/exit-code harnesses, and hardened the
  cli-a json/html/terminal/stats regen to ALWAYS write the in-repo vendored override
  (never al-sem) and only when the engine output differs from the resolved baseline,
  keeping the vendored set minimal. al-sem stays FROZEN; no L2/L3 ripple this iteration
  (the L2/L3rt differential is byte-identical); no symbol-reader/cache surface moved
  (`cli_c_cache` green) → no cache-version bump; `KNOWN_DIVERGENCES.json` stays `[]`.
- Rebaselined the in-repo differential goldens after the G-1..G-12 detector-gap fixes.
  Two content classes moved: (a) **G-4** d1 transitive-loop `rootCause` text now names
  the terminal routine ("… reaches <op> in Z, which has no loop of its own — the
  operation runs once per iteration of that loop.") on `ws-d1` (r4) and
  `ws-d1-multi-caller` (r4 / cli-a json+html+terminal / gate-sarif) — a field-level
  change to `rootCause` only; presence, severity, ids, rootCauseKeys, and fingerprints
  are byte-identical. (b) **G-12** d3 now suppresses the PK-only existence-check `Get`
  in `ws-inline-suppress`'s `UnsuppressedD3`; the gate-suppress anti-degenerate witness
  was preserved by editing that fixture so the routine reads a Normal field (`Name`)
  after the `Get`, yielding a genuine d3 finding — gate-suppress SARIF/PR-summary and
  the `ws-inline-suppress` L2 feature golden were rebaselined accordingly. Added
  `REGEN_TEMP_GOLDENS` regen branches to the gate-suppress and L2-features differential
  harnesses (mirroring the existing gate-sarif / cli-a / r4 / l3rt regen paths). No
  symbol-reader/cache surface moved (`cli_c_cache` green) → no cache-version bump;
  `KNOWN_DIVERGENCES.json` stays `[]`.

### Fixed
- Detector-audit class A + Singleton BUG-5 (docs/detector-audit.md):
  `d4-repeated-lookup-in-loop` fixed on two fronts. (1) **Temp gate** — a repeated
  identical lookup on a provably `temporary` record (`temp_state` Known(true)) is
  an in-memory read with no SQL round-trip to hoist and no longer fires (same
  `is_known_temp` gate as d1/d2/d33; new `tempRecord` skip stat).
  Suppression-direction exact: the same shape on a physical record still fires
  (control in `tests/gap_audit_d4.rs`). (2) **BUG-5 duplicate finding id** — the
  id `d4/{routine}/{loop}/{varLower}` omitted the literal lookup key, so two
  distinct keys each repeated 2+ times on the same (routine, loop, variable)
  produced colliding ids. The literal key is now appended to the id ONLY when a
  variable has multiple qualifying key groups, so single-key findings keep their
  pre-fix ids byte-identical (existing d4 goldens verified unmoved, r4
  differential green).
- Detector-audit classes A + C (docs/detector-audit.md): `d2-event-fanout-in-loop`
  no longer false-fires when an event subscriber's in-loop db ops are all
  structurally non-actionable. Three guards now mirror d1's terminal/op selection:
  (1) **Next-terminator (G-1)** — a subscriber's own `until <var>.Next() = 0`
  terminator is the loop's cursor advancement, not a db op; (2) **virtual/system
  table (G-6)** — a subscriber reading `AllObjWithCaption`/`Field`/`Integer`/… hits
  the platform's in-memory metadata store, not SQL; (3) **temporary record** — an op
  provably on a `Known(true)` temporary record does no physical-db work (mirrors
  d33's temp gate). The three filters are applied in `D2Policy::terminals_at` (so
  transitive callees are covered too), and the `any_db_subscriber` aggregation now
  keys off the supplementary walk yielding a Complete path to a SURVIVING db op — so
  a subscriber touching ONLY terminator/virtual/temp ops is no longer counted as a
  db subscriber. The `is_terminator_next` / `is_known_temp` helpers were promoted
  from d1.rs to `detectors/mod.rs` (`pub(crate)`) for reuse; d1 imports them
  unchanged. Suppression-direction exact: a REAL db op (e.g. `Modify` on a physical
  record) inside a subscriber loop still fires (control in
  `tests/gap_audit_d2_guards.rs`).
- Detector-audit class B (docs/detector-audit.md): d21/d37/d39 no longer false-fire
  on the implicit `Rec` inside table-LEVEL `OnInsert`/`OnModify`/`OnDelete`/`OnRename`
  triggers, where the AL platform loads `Rec` before the trigger body runs AND
  auto-persists it afterwards (`OnInsert`/`OnModify`/`OnRename` write `Rec` to the
  table; `OnDelete` deletes it, making "validate without persist" moot). The
  `is_platform_loaded_trigger_rec` gate's `Table`/`TableExtension` arm (previously
  field-level `OnValidate` only) now also recognizes those four table-level trigger
  names — covering d21 (read-without-load), d37 (validate-without-persist), and
  d11 which share the gate — and a new `is_auto_persist_trigger_rec` signal makes
  d39 (record-left-dirty-across-chain) skip a table-level trigger caller that
  forwards `Rec` by-var to a dirty helper (new `autoPersistTriggerRec` skip stat).
  Suppression-direction exact: trigger kind + Table/TableExtension object +
  receiver `Rec` only — the same ops in a non-trigger procedure or on a non-`Rec`
  record inside the trigger still fire (controls in
  `tests/gap_audit_b_table_triggers.rs`; G-9/G-14 page/field-trigger behavior
  unchanged).
- G-19 (docs/engine-gaps.md): d1/d3/d10 no longer fire on a keyword-less by-`var`
  `Record` parameter of a **`local`** procedure when its temporariness is
  CLOSED-WORLD PROVEN: the routine is `local` (AL language rule — callable only
  within its owning object), every same-object call site that could name it is
  resolved (no parse-incomplete sibling bodies, no unresolved or unclassifiable
  name-matching calls), it has at least one resolved caller, every caller edge is
  a binding-carrying kind (`direct`/`method`), and every caller's argument
  binding for that parameter is `Known(true)` temporary — directly or
  recursively through another closed-world-proven `local` forwarding parameter
  (cycles ground to NOT-proven). New `engine::l5::closed_world_temp` module
  computes the proven `(routineId, paramIndex)` set once in the detector
  context; the d3/d10 temp gates consult it next to the existing `Known(true)`
  gate, and d1's per-path resolver
  (`resolve_temp_along_path_closed_world`) resolves a proven PD frame to
  `Known(true)` — so the intra-callee shape downgrades to `info` exactly like
  any other proven-temp record (~12 CDO false positives: GetUpgradeData,
  MergePdfInBatches/ProcessMergeBatch Temp Blob, TempAut*). Suppression-
  direction safe — every uncertainty fails the proof and keeps firing:
  public/internal routines (open world), any physical/unknown caller argument,
  unresolved same-object name-matching calls, dynamic/interface/event edges,
  event subscribers and triggers (runtime-invoked), zero-caller dead locals
  (no vacuous proof), and RE-11 colliding routine ids. The open-world shapes'
  recommended SOURCE fix remains adding the `temporary` keyword to the
  parameter (contract-trust `Known(true)` — covered by a regression guard).
  Tests: `tests/gap_g19_temp_param.rs` (proof + 7 firing controls + keyword
  guard); `temp_state_path` / `temp_state_substitution` /
  `temp_state_param_forwarding` / `gap_g13_temp_gate` stay green.
- G-18 (docs/engine-gaps.md): `d1-db-op-in-loop` no longer attributes a loop to an
  op when the loop is on a SIBLING call path, not on the actual path to the op.
  Root cause: the internal routine id (`compute_routine_id`) carries no member
  discriminator, so two same-name same-signature triggers in one object (e.g. two
  page actions, each `trigger OnAction()`) collide on the id — and with it every
  derived call-site id (`{rid}/cs{n}`). The combined graph files BOTH bodies'
  edges under the one shared `from` key, and d1's root-edge lookup (by callsite id
  alone) could pick the SIBLING action's edge for the LOOPING action's in-loop
  call site — walking a straight-line chain the loop is not on (the CDO batch-7
  `eDocumentsConfigExists` IsEmpty ×2 false positives, loop mis-attributed from a
  separate `RunReport`-style looping action). d1's root-edge match now also
  requires the edge's TARGET routine to carry the call site's own callee name
  (`edge_target_matches_callsite_callee`): the resolver is name-keyed, so a
  genuinely-own `direct`/`method` edge always matches — the guard only ever
  filters cross-body edges under a colliding id and can never suppress a genuine
  transitive finding (un-nameable object-run/unknown callees and out-of-source
  targets are accepted unchanged; implicit-trigger edges never reach the guard —
  their callsite ref is an op id). A real in-loop chain THROUGH a colliding
  trigger and the vanilla transitive shape both keep firing at unchanged severity
  (`tests/gap_g18_transitive_loop.rs`); `gap_g1`/`gap_g4` stay green. The
  underlying routine-id collision itself (which also conflates `routine_by_id` /
  `call_site_by_id` views for colliding triggers) is documented in
  docs/engine-gaps.md G-18 as residual follow-up.
- G-17 (docs/engine-gaps.md): `d33-unfiltered-bulk-write` no longer fires when the
  filter was provably applied by (a) an in-source helper defined ON the receiver's
  own TABLE — the real-world G-3 miss: `LineReport.SetEMailTemplateLineFilter(Rec);
  LineReport.DeleteAll();` passes the filter-VALUE source by value while the helper
  filters its implicit self record (bare `SetRange(...)` in a table method), a shape
  G-3's by-`var`-argument summary could never match because the call resolver's
  `parse_object_type_ref` has no `Record` keyword, so record-receiver member calls
  never resolve to table procedures (the G-3 root cause). The G-3 gate
  (`record_filtered_by_call_before` in `src/engine/l5/detectors/mod.rs`) now adds a
  receiver-method tier that joins receiver-var `table_id` → in-source table
  procedure by name (ALL same-name candidates must net-filter the implicit self —
  last `SetRange`/`SetFilter`/`Reset` event on the self, as bare calls,
  `Rec.`-member calls, or `Rec` record ops, must be a filter); and (b) the page
  builtin `CurrPage.SetSelectionFilter(<var>)` (matched structurally: a member call
  to `SetSelectionFilter` whose bound argument is the bulk-op record — the platform
  copies the page's row selection onto it as filters). Suppression-direction safe:
  no-filter, non-filtering receiver method, receiver method whose net effect is
  filter-then-`Reset`, and `SetSelectionFilter` on a DIFFERENT record all keep
  firing (`tests/gap_g17_d33_filters.rs`); `tests/gap_g3_interproc_filter.rs` stays
  green. TableExtension-defined helpers and dependency-table helpers stay
  unrecognized (conservative; the ABI side is G-17's deferred lower-priority part).
- G-16 (docs/engine-gaps.md): `d11-modify-without-get` / `d21-read-without-load` no
  longer fire "never loaded" when the record provably was. Two extensions of G-10,
  both suppression-direction safe: (a) the callee-load summary
  (`record_loaded_by_call_before` in `src/engine/l5/detectors/mod.rs`) now follows a
  BOUNDED multi-hop wrapper chain (`MAX_LOAD_WRAPPER_HOPS = 3` callee hops) — every
  hop is the same resolved-binding by-`var` join as G-10, so
  `FindTemplate -> FindTemplateWithReportID -> FindSet`, forwarded boolean facade
  loaders, and `GetBySystemId` inside a wrapper now count, while a load 4+ hops down,
  an unresolved callee, a by-value binding, or a chain that only filters all keep
  firing (Get-or-Insert facades like `InsertIfNotExists` were already covered at one
  hop since `Init`/`Insert` are recognized load ops). (b) NEW record-assign-as-load
  gate `record_loaded_by_assignment_before`: a whole-record assignment
  `RecB := RecA` strictly before the op loads `RecB` when `RecA` is provably loaded
  AT the assignment point — a recognized load op / loading call before it, the
  platform-loaded trigger `Rec` (G-9), a parameter record (the detectors' own
  caller-loaded skip), or a further assignment from a loaded var (chain bounded at
  `MAX_ASSIGN_CHAIN_DEPTH = 3` links). Backed by a new internal-only
  `PVarAssignment.rhs_identifier` (serde-skipped like G-1's `in_until_condition`,
  excluded from `PartialEq` — L2 feature goldens stay byte-identical) that is set
  ONLY when both assignment sides are bare identifiers, so field writes and
  expression RHS never suppress. Controls in `tests/gap_g16_deep_wrappers.rs` prove
  no-load, deep-non-loading-chain, beyond-bound-load, assign-from-unloaded,
  assign-after-op, and RHS-loaded-after-assignment all still fire;
  `tests/gap_g10_load_wrappers.rs` stays green.
- G-15 (docs/engine-gaps.md): `d3-missing-setloadfields` no longer fires when the fields
  touched after a retrieval are only WRITTEN, and `d42-cross-call-wrong-setloadfields`
  no longer counts PRIMARY-KEY fields as must-be-loaded. Three exact sub-class
  suppressions, everything else keeps firing: (a) a field access whose source position
  AND member name match a recorded assignment LHS (`PVarAssignment` is anchored at the
  statement start, which IS the LHS member expression's start) is a WRITE target —
  writes need no SetLoadFields, so they no longer count toward d3's
  "accessed-without-load" witness (RHS reads sit at different positions and keep
  counting); (b) an intervening `Init()` record op or `Clear(<var>)` bare call between
  the retrieval and the access closes d3's access window (new `WINDOW_CLOSING_OPS` —
  the access reads the re-initialised buffer, not the loaded row; `deriveLoadStates`'s
  `INVALIDATING_OPS` is unchanged since `Init` does not clear the SetLoadFields
  selection); (c) d42 now drops the callee parameter table's PK (first key) fields from
  `requiredLoadedFieldsAtEntry` — the PK is always loaded regardless of SetLoadFields —
  reusing G-12's d3 exclusion via the new shared `primary_key_field_names_lc` +
  `normalize_load_field_arg` helpers in `src/engine/l5/detectors/mod.rs` (new `pkOnly`
  skip counter). Genuine reads of non-PK normal fields still fire (controls in
  `tests/gap_g15_d3_d42_writes.rs`; `tests/gap_g12_d3_refinements.rs` stays green).
- G-14 (docs/engine-gaps.md): `d11-modify-without-get`, `d21-read-without-load`, and
  `d37-validate-without-persist` no longer fire on the implicit `Rec` inside page field
  `OnLookup` / `OnAssistEdit` triggers — the G-9 trigger set
  (`PAGE_TRIGGERS_REC_LOADED` in `src/engine/l5/detectors/mod.rs`) missed the two
  field-level lookup triggers even though the AL platform loads `Rec` before they run
  and the page framework persists a `Validate` performed inside `OnLookup`. The gate
  stays exact and structural (trigger kind + Page/PageExtension + receiver `Rec`);
  non-trigger procedures and non-`Rec` receivers keep firing (controls in
  `tests/gap_g14_onlookup_triggers.rs`). No golden moved.
- G-13 (docs/engine-gaps.md): `d10-self-modifying-loop` and `d39-record-left-dirty-across-chain`
  no longer fire on `Known(true)` TEMPORARY records — they were never added to the temp-state
  epoch's gate set (d1/d3/d33/d36/d37/d40 were). d10 now skips a mutating op on the iterating
  record when `op.temp_state` is Known(true) (same gate as d33): an in-memory cursor self-modify
  is safe — cursor corruption only applies to physical SQL cursors. d39 now skips a forwarded
  binding when `binding.source_temp_state` is Known(true) (same gate as d40): a temporary record
  left Validate-dirty across a helper chain has no SQL consequence. Both gates are exact-match
  on Known(true) — physical and Unknown records keep firing (suppression-direction safe; proven
  by controls in `tests/gap_g13_temp_gate.rs`). Both detectors gain a `tempRecord` skip counter.
- G-8 (docs/engine-gaps.md): a codeunit-global `temporary` record FORWARDED by-var into a
  helper (e.g. `TempErrors: Record "Error Message" temporary;` passed to a local
  `LogError(var Errors: Record ...)` that does the db op) no longer resolves "temp state
  uncertain". Root cause: the L2 argument-binding builder only matches the routine's OWN
  params/locals, so an arg naming an object-global record var was emitted
  `sourceKind: "unknown"` with NO `sourceTempState` — both the L4 PD substitution
  (`substitute_pd_temp_state`) and the L5 per-path resolver (`resolve_temp_along_path`)
  collapse a missing binding source to `Unknown`, so the helper's PD op stayed
  "uncertain" even though the global carries the exact structural `temporary` keyword.
  Fix (`src/engine/l3/l3_workspace.rs`, inside the existing RV-8 relabel block, AFTER the
  Task-3 global promotion): backfill an `"unknown"` binding whose arg text is a BARE
  identifier matching a promoted-global record var — and whose innermost declaration IS
  that global (a same-named scalar param/local shadows it → skipped, conservative) — with
  `sourceKind: "global"`, the promoted per-routine record-var id, and the global's own
  `tempState` (Known(true) only ever from the `temporary`-keyword signal Task 3 captured;
  a NON-temp global backfills Known(false) and keeps firing). Direct ops on globals
  (Task-3 promotion), keyword-temp by-var params (Task 8 / RV-3 contract-trust), and the
  keyword-less by-var PD-at-path-root → Unknown behaviour were verified CORRECT and are
  regression-guarded. Tests: `tests/gap_g8_residual_temp.rs` (forwarded temp global →
  info, forwarded non-temp global keeps firing, plus the Case A/B ground-truth guards).
  No in-repo golden moved (no golden fixture forwards an object-global record var).

### Changed
- G-7 (docs/engine-gaps.md): `d1-db-op-in-loop` findings whose EVERY path root routine is
  provably dead are now DOWN-CONFIDENCED — confidence drops one notch (likely → possible)
  and the rootCause gains "(looping routine appears unreachable from any entry point; see
  d14-dead-routine)" (CDO triage batch 4 — `UpgradeOutputProfileOnDocsWorker`, whose only
  caller is commented out). Deliberately NOT suppression: d14's dead-determination has its
  own open-world false positives (the engine is source-only — reflection-style invocation,
  unmodeled dispatch), so the finding KEEPS FIRING at the same severity, id, rootCauseKey,
  and fingerprint (the fingerprint hashes the rootCauseKey, not the rootCause text or
  confidence — suppression baselines are unaffected). The dead signal is d14's EXACT
  emission criteria, factored into the shared `provably_dead_routine_ids` /
  `classify_routine` (`src/engine/l5/detectors/d14.rs` — forward-BFS unreachable from the
  entry-point closure + `local`/app-scoped-`internal` access + not a Test object + not a
  property-expression host + not itself a root); d14's own output and stats are
  byte-unchanged by the refactor. The check runs POST-merge across ALL merged paths
  (canonical + additionalPaths): any live — or merely unprovable (public, Test object,
  page-hosted) — path root keeps full confidence. New d1 stats bucket
  `downConfidencedDeadRoutine`. d1 only for now (the gap's evidence is d1-only; other
  detectors can adopt the shared helper if triage shows volume). Covered by
  `tests/gap_g7_dead_routine.rs` (down-confidence + firing/severity preservation + live /
  public / mixed-live-and-dead controls). Moves d1 confidence/rootCause text and the d1
  stats shape in r4/cli-a/gate goldens only for dead-rooted fixtures; rebaseline deferred
  to the consolidated gap-fix rebaseline task.
- G-4 (docs/engine-gaps.md): `d1-db-op-in-loop` PURE-TRANSITIVE findings — the terminal
  op's own routine has NO loop around the op; the loop lives purely in an ancestor — now
  say so explicitly. The rootCause names the terminal routine and attributes the loop to
  the ancestor: `"A loop in X reaches <Op> on <Table> in Z, which has no loop of its own —
  the operation runs once per iteration of that loop."` (previously the terminal routine
  was never named, so the text read as if the op's own routine looped — CDO triage
  batches 7, 10). WORDING ONLY, deliberately NOT suppression: these findings are
  genuinely real (the op runs once per ancestor iteration — real SQL cost), so presence,
  severity, confidence, ids, rootCauseKeys, and fingerprints are all unchanged; a direct
  in-loop op and a transitive terminal op sitting inside the CALLEE's own loop keep the
  original wording byte-identical. The optional confidence-notch lowering was skipped
  (wording-only, per the gap's conservative scope). Covered by
  `tests/gap_g4_transitive_wording.rs` (new wording + firing/severity preservation +
  both unchanged-wording controls). Moves the d1 rootCause TEXT in r4/cli-a/gate-sarif
  goldens for transitive fixtures (`ws-d1`, `ws-d1-multi-caller`); rebaseline deferred to
  the consolidated gap-fix rebaseline task (field-level diff confirms only `rootCause`
  diverges).

### Fixed
- G-5 (docs/engine-gaps.md): findings no longer render the WRONG table name in their
  rootCause when a `tableextension`'s OWN object number collides with a real table's
  number in the same app (CDO triage batches 2, 3 — ops on `MergeTableTopBottom` /
  `HtmlTableStyle` / `HtmlTableStyleLine` reported as `CDOReturnShipmentHeader` /
  `CDOPurchaseReceiptHeader` / `CDOJobExt`, which are tableextension NAMES). Root cause:
  a `tableextension` declaration is indexed as an `L3Table` stub whose internal id reuses
  the EXTENSION's object number (`${appGuid}/table/${extNumber}` — kept so
  `merge_extension_fields` can find the extension's fields), so it COLLIDES with a real
  table sharing that number and clobbered it in every LAST-wins id lookup
  (`describe_table` tier 1 then rendered the extension's name). Fix: new
  `L3Table::is_extension_stub` marker + REAL-over-stub collision preference in every
  table lookup map — `SymbolTable` (`tables_by_name`/`tables_by_id`), the shared
  `table_by_id_preferring_real` helper consumed by `DetectorContext::table_by_id` (both
  source-only and cross-app builds), the HTML formatter's table-label map, and the policy
  engine's `tables_by_id`. Within the same kind (real/real, stub/stub) LAST-wins is
  preserved (al-sem parity); the `merge_extension_fields` algorithm itself is untouched
  (stays in lockstep with its projected twin). Name-correctness only: finding presence,
  severity, ids, and fingerprints are unchanged (the op's `table_id` STRING is identical —
  only the rendered name was wrong). Covered by `tests/gap_g5_wrong_table_name.rs`
  (collision repro in both assembly orders + sequential/transitive multi-subloop
  regression guards). No in-repo golden moved; the real-app (CDO) rebaseline remains with
  the consolidated gap-fix rebaseline task.
- G-3 (docs/engine-gaps.md): `d33-unfiltered-bulk-write` no longer fires on a
  `DeleteAll`/`ModifyAll` whose receiver was provably filtered by a helper procedure call
  earlier in the routine (CDO triage batches 9, 10 — `SetTemplateFilter(Rec)`,
  `SetMergeFieldFilter(Rec)`-style helpers, ~5 FPs). Implemented as
  `record_filtered_by_call_before` (`src/engine/l5/detectors/mod.rs`), the filter analog of
  G-10's load gate, consulted by d33 after its intraprocedural `was_filtered_before` scan.
  It REUSES the G-10 one-hop callee-summary join — extracted into the shared
  `callee_applies_op_to_by_var_arg` helper (resolve the callsite's callee via
  `resolved_call_edge_by_callsite`, join `argument_bindings` with
  `upgraded_bindings_by_callsite` requiring `binding_resolution == "resolved"` +
  `callee_parameter_is_var`, then inspect the callee's `record_operations` on the by-var
  parameter) — with a filter predicate: the callee's NET effect on the parameter must be
  filtered, i.e. its last `SetRange`/`SetFilter`/`Reset` op (by source position) on that
  parameter is a filter (`RECORD_FILTER_OPS` — the exact set d33 applies intraprocedurally,
  now shared), not a `Reset`. A caller-side `Reset` between the helper call and the bulk op
  also voids that call (mirrors `was_filtered_before`'s Reset semantics). One hop only;
  suppression-direction safe: no filter call, a non-filtering callee, a by-value binding,
  an unresolved callee, a filter call AFTER the bulk write, a callee that filters then
  Resets, and a caller-side Reset after the helper all keep firing. Covered by
  `tests/gap_g3_interproc_filter.rs` (helper-SetRange + helper-SetFilter suppressions; six
  controls). No in-repo golden moved by this change (full `cargo test` divergence-checked);
  the real-app (CDO) rebaseline remains with the consolidated gap-fix rebaseline task.
- G-10 (docs/engine-gaps.md): `d11-modify-without-get` / `d21-read-without-load` no longer
  fire when the record WAS loaded by a call that isn't a literal `Get`/`Find` record op
  (CDO triage batches 1, 10, 11, 12 — `GetBySystemId` ×4, `FindTemplate`-style wrappers,
  `InsertIfNotExists`, var-out facade loaders). Two structural tiers, both implemented in
  the shared `record_loaded_by_call_before` gate (`src/engine/l5/detectors/mod.rs`),
  consulted by d11/d21 after their intraprocedural `loaded_before` scan: (1) **platform
  built-in loaders** — a member call `<var>.GetBySystemId(...)` strictly before the
  mutating/reading op counts as a load (exact-name allowlist `PLATFORM_LOADER_METHODS`,
  case-insensitive, receiver must match the record variable exactly; `GetBySystemId` is
  not in the L2 record-op map so it surfaces as a call site, invisible to the old scan);
  (2) **one-hop callee load summary** — when the record was passed as an argument whose
  binding RESOLVED to a by-`var` record parameter of a workspace callee
  (`resolved_call_edge_by_callsite` + `upgraded_bindings_by_callsite`, the same join
  d37/d39/d40 use), and that callee's own body performs a recognized load op
  (`RECORD_LOAD_OPS` — the exact set d11/d21 apply intraprocedurally, now shared) on that
  parameter, the record is loaded after the call. This covers custom `FindXxx`/`GetXxx`
  wrappers, `InsertIfNotExists` (Insert is a recognized load), and var-out facade loaders
  in one mechanism, and is the load analog of G-3's planned filter summary (one hop, callee
  body only, reusable pattern). Suppression-direction safe: an unresolved callee, a
  by-value binding (the callee loads its own copy), a different variable, a non-loading
  callee, or a call AFTER the op all keep firing. Covered by
  `tests/gap_g10_load_wrappers.rs` (GetBySystemId + by-var helper-load suppressions for
  both detectors; controls: no load, load after the op, load on a different record,
  filter-only callee, by-value callee load, unresolved callee — all still fire). No
  in-repo golden moved by this change (full `cargo test` divergence-checked); the
  real-app (CDO) rebaseline remains with the consolidated gap-fix rebaseline task.
- G-2 (docs/engine-gaps.md): runtime-implied tempness is now inferred from the exact
  `not IsTemporary → Error` structural guard, removing the dominant post-epoch temp-related
  FP class (CDO triage batches 1, 9, 11 — ~15 FPs: `CDO File` ops, `EmbedFiles`,
  `UpdateFromXml`, signature templates). Two sub-features, both AST shape matches (no
  string-sniffing, no dataflow): (1) **self-guarding temp table** — a table whose
  OnInsert/OnModify/OnDelete/OnRename trigger contains a TOP-LEVEL
  `if not Rec.IsTemporary[()] then Error(...)` guard is temporary BY RUNTIME CONTRACT
  (every instance errors otherwise), so `index_table` now sets `L3Table.is_temporary`
  exactly like `TableType = Temporary` and the existing table-level override upgrades all
  ops on it to `Known(true)`; (2) **entry-guard temp routine** — a routine whose FIRST
  executable statement is `if not <X>.IsTemporary[()] then Error(...)` where `<X>` is a
  record var/param (incl. promoted globals) or the implicit `Rec`/`xRec` proves `<X>`
  temporary for the whole body (the guard dominates it), captured at L3 assembly as
  `L3Routine.entry_temp_guard_receiver` and applied as a new override pass in
  `record_types.rs` (after var/op temp derivation, alongside the table-level override).
  The guard matcher (`is_temporary_error_guard` in `l3_workspace.rs`) accepts only the
  exact shape: an `if` with NO else whose condition is `not <recv>.IsTemporary[()]` (or
  `<recv>.IsTemporary[()] = false`) with a bare-identifier receiver and a zero-argument
  IsTemporary, and whose then-branch is an `Error(...)` call (directly or a
  `begin Error(...); end` block with exactly that one statement). Suppression-direction
  safe — both signals PROVE tempness (the code errors at runtime otherwise), upgrades are
  purely additive toward `Known(true)`; any deviation (guard not the first statement,
  nested/non-top-level table guard, non-negated condition, `exit` instead of `Error`)
  leaves the state untouched → detectors keep firing. Covered by
  `tests/gap_g2_runtime_temp.rs` (table-contract resolution + d1 downgrade, paren-less +
  OnDelete variants, entry-guard param resolution + d33 suppression on a guarded global;
  controls: plain table, non-negated trigger, unguarded routine, guard-not-first,
  exit-then-branch — all keep firing). No in-repo golden moved by this change (no fixture
  contains an IsTemporary guard); the real-app (CDO) rebaseline remains with the
  consolidated gap-fix rebaseline task.
- G-12 (docs/engine-gaps.md): `d3-missing-setloadfields` no longer fires on four clean FP
  sub-classes from the CDO triage (batches 1, 8, 10/12). The "unloaded fields accessed"
  computation now (1) excludes the table's PRIMARY-KEY fields (first key — `L3Table.keys[0]`
  member names; the PK is always loaded regardless of SetLoadFields), (2) excludes
  **FlowField** fields (`field_class == "FlowField"` — an uncovered FlowField read needs
  `CalcFields`, d22's domain, not `SetLoadFields`), and (3) consequently suppresses the
  existence-check shapes (`exit(Rec.Get(...))`, `if Rec.Get(...) then exit;` + Init/PK-write/
  Insert) where no normal field is read after the Get — the accessed set is empty, so there is
  no witness. (4) The missed pre-Get `SetLoadFields` was a quote-normalization gap, not an
  ordering gap: `derive_load_states` already walks ops in source order, but the L2 body walk
  records `SetLoadFields("Unit Price")` arguments with their quotes while field accesses are
  stored unquoted, so a quoted load argument never covered the later access — load-set
  arguments are now trimmed + outer-quote-stripped + lowercased (`normalize_load_field_arg`)
  for `SetLoadFields`/`AddLoadFields`. Suppression-direction safe: only PK / FlowField names
  resolved against the table model are excluded (unresolved names stay in the accessed set),
  a Get reading BOTH a PK and an uncovered normal field still fires (missing list names the
  normal field only), and quote normalization only ever ENLARGES coverage matching (fewer
  false "incomplete"s, never a new finding). Covered by `tests/gap_g12_d3_refinements.rs`
  (PK-only, FlowField-only, two existence-check shapes, quoted+plain pre-Get SetLoadFields
  suppressions + uncovered-read, PK+normal, FlowField+normal, incomplete-pre-Get controls
  that must keep firing). In-repo gate/r4 goldens with d3 findings may move only where a
  finding's premise no longer holds — the real-app (CDO) rebaseline remains with the
  consolidated gap-fix rebaseline task.
- G-6 (docs/engine-gaps.md): SQL-cost detectors no longer fire on ops targeting BC
  VIRTUAL/system tables (`AllObj`, `AllObjWithCaption`, `Field`, `Key`, `Object`,
  `Object Metadata`, `Table Metadata`, `Page Metadata`, `Codeunit Metadata`,
  `Report Metadata`, `Database Locks`, `Session`, `Active Session`, `Integer`, `Date`) —
  these have NO physical SQL backing (they read the platform's in-memory metadata store),
  so an in-loop read of one is never a SQL round-trip (CDO triage batch 5, 6 FPs:
  `AllObjWithCaption`/`Field` reads in loops flagged "type not loaded"). The suppression is
  a shared exact-name gate (`VIRTUAL_SYSTEM_TABLES` allowlist + `is_virtual_system_table` +
  `op_targets_virtual_system_table` in `src/engine/l5/detectors/mod.rs`, same pattern as
  G-9's `is_platform_loaded_trigger_rec`): the op's type did NOT resolve to a workspace
  table (a user table with a colliding name is physical → keeps firing) AND the record
  variable's DECLARED type name matches the allowlist exactly (case-insensitive). Consulted
  by `d1-db-op-in-loop` (direct in-loop branch — new `virtualTable` skip stat, present only
  when non-zero — AND `terminals_at`, so virtual ops no longer fire transitively from an
  ancestor loop) and `d4-repeated-lookup-in-loop` (candidate filter). `d3`/`d33` need no
  gate: they already bail on unresolved-table ops, and a virtual table never resolves in the
  source-only workspace. Suppression-direction safe: only the exact-name allowlist is
  skipped; a loaded physical table and a NOT-loaded table with any other name keep firing.
  Covered by `tests/gap_g6_virtual_tables.rs` (d1 direct + transitive suppression, d4
  suppression, loaded-physical / unloaded-non-virtual / repeated-normal-lookup controls).
  No in-repo golden moved — full `cargo test` is green (no fixture performs record ops on a
  virtual table); the real-app (CDO) rebaseline remains with the consolidated gap-fix
  rebaseline task.
- G-11 (docs/engine-gaps.md): `d20-unreachable-after-exit` no longer fires when the only
  thing after an unconditional `exit(...)`/`Error(...)`/`CurrReport.Quit` is comment or
  pragma trivia — `exit(0); // note` (trailing inline comment), an own-line comment after
  the exit, and the comment-trailed single-line / conditional-fall-through exit shapes from
  the CDO triage (~6 FPs, batches 4/7/11/12) all stop firing. Root cause: the L2
  unreachable-after-exit scan (`src/engine/l2/body_walk.rs`, code_block entry) collected
  `named_children` as "statements", and in the V2 grammar `comment` / `multiline_comment` /
  `pragma` nodes are named children of `code_block` — so a comment was flagged as the "next
  statement" after the exit. The scan now filters that trivia out, so d20 fires ONLY when
  the terminator is unconditional AND an actual executable statement follows it in the same
  block. The other two triaged shapes were already structurally correct in the Rust engine
  (a bare single-line `exit(expr)` body has no following sibling; a conditional
  `if … then exit(x)` sibling is an `if_statement`, which `unconditional_exit_kind` never
  classifies) — locked in by tests. Suppression-direction safe: a REAL statement after an
  unconditional exit still fires, including when a comment sits between the exit and the
  dead statement. Covered by `tests/gap_g11_d20_position.rs` (trailing/own-line comment,
  single-line body, conditional fall-through suppressions + unconditional-exit,
  unconditional-Error and comment-between controls that must keep firing). No in-repo
  golden moved — full `cargo test` is green (no fixture exercises a comment-after-exit
  shape); the real-app (CDO) rebaseline remains with the consolidated gap-fix rebaseline
  task.
- G-1 (docs/engine-gaps.md): `d1-db-op-in-loop` no longer fires on the `Next()` that IS the
  `until <var>.Next() = 0` terminator of the very loop being iterated — that `Next()` is the
  loop's own per-iteration cursor advancement (removing it breaks the loop), never an
  actionable db op (the single largest crit/high FP class in the CDO triage, ~15+ FPs). The
  suppression is an exact structural proof: the L2 body walk now marks a record op whose node
  sits inside the `condition` field of its NEAREST enclosing `repeat_statement`
  (`PRecordOperation.in_until_condition`, serde-skipped so every feature-level golden stays
  byte-identical; forwarded through `L3RecordOperation`), and d1 skips
  `op == "Next" && in_until_condition` in BOTH its direct in-loop branch and `terminals_at`
  (so a callee's own terminator no longer fires transitively from an ancestor loop either).
  Suppression-direction safe: only a proven terminator `Next` is skipped — a real db op in
  the loop body, a mid-body `Next()` advancing a DIFFERENT cursor, and the cursor-opening
  `FindSet` inside an outer loop all keep firing (no non-Next op is ever suppressed). Covered
  by `tests/gap_g1_next_terminator.rs` (terminator suppression — direct, nested-opener and
  transitive — plus in-body Modify and second-cursor Next controls). No in-repo golden moved:
  the direct terminator-Next was already absent from every fixture golden (the pre-existing
  pre-loop cursor-opener heuristic covered the simple `FindSet → repeat → until Next` shape)
  and no fixture exercises the transitive/nested-opener shapes; the real-app (CDO) rebaseline
  remains with the consolidated gap-fix rebaseline task. The L2 baseline-vector comparison
  (`tests/l2_vectors.rs`) compares the serialized contract surface only — `PRecordOperation`
  gained a manual `PartialEq` that excludes the serde-skipped internal flag.
- G-9 (docs/engine-gaps.md): `d11-modify-without-get`, `d21-read-without-load` and
  `d37-validate-without-persist` no longer fire on the implicit `Rec` inside page triggers
  (`OnValidate`, `OnAction`, `OnAfterGetRecord`, `OnDrillDown`, `OnAfterGetCurrRecord`) or
  table field `OnValidate` triggers — the AL platform has already loaded `Rec` before those
  triggers run, and a field `OnValidate` calling `Validate(...)` on a sibling field is normal
  field-chain validation whose persistence is the caller's job (the single largest medium/low
  FP class in the CDO triage, ~40+ FPs). The suppression is an exact structural gate
  (`is_platform_loaded_trigger_rec` in `src/engine/l5/detectors/mod.rs`): routine
  `kind == "trigger"` + owning object type Page/PageExtension (page trigger-name set) or
  Table/TableExtension (`OnValidate`) + op receiver `Rec` (case-insensitive); anything
  uncertain keeps firing (suppression-direction safe). Each detector reports the skip under
  a new `triggerRec` stats key (omitted when zero, so existing stats output is unchanged).
  Covered by `tests/gap_g9_trigger_rec.rs` (page-trigger + table-field-trigger suppression,
  plus non-trigger and non-Rec controls that must keep firing). No in-repo golden moved —
  no r4/cli/r3a fixture exercises trigger-Rec for these detectors.

### Added
- Metamorphic soundness oracle for the temp-state epoch (Task 14 / ts14 — RV-2, the
  mechanical guard for the whole epoch's suppression direction; `tests/temp_state_oracle.rs`).
  The oracle encodes the governing property: adding the `temporary` modifier to a record
  declaration can only make that record MORE temporary, so the analyzer's findings may only
  be REMOVED or DOWNGRADED under the edit — never ADDED, never UPGRADED — with ONE carve-out
  (RV-1): FlowField `CalcFields`/`SetAutoCalcFields` findings are INVARIANT (a temp record's
  FlowField still evaluates its CalcFormula against the physical flow targets, a real SQL
  round-trip, so they must keep firing at the same severity). For each of five standalone
  inline fixtures (DeleteAll buffer, Modify-in-loop, Blob CalcFields, FlowField CalcFields,
  and a Get/Modify physical-op control) it runs the FULL default detector set in-process
  (`assemble_and_resolve_default` + `run_detectors`) over the ORIGINAL source and over a
  mechanically `temporary`-edited copy (the edit appends ` temporary` to the targeted
  `Record "Name"` declaration, shifting no later anchor), then compares the two `Finding`
  sets by a stable `(detector, file, line, col)` key: suppression fixtures must show edited
  ⊆ original under "removed or downgraded" (and must actually soften); the FlowField fixture
  must be byte-identical (key + severity). A corpus-wide guard asserts no addition / no
  upgrade across every fixture. Purely additive (new test file, no `src` change, no golden
  movement); a red here is a genuine product-soundness signal, not a golden to refresh.
- RecordRef `GetTable` / `OpenTemporary` local-only `tempState` derivation (Task 12 / ts12,
  Component 4 / G6). The L3 record-type resolution pass now derives a `RecordRef` variable's
  `tempState` from two structurally deterministic call patterns — `RecRef.Open(no, true)`
  (OpenTemporary form → `Known(true)`), `RecRef.Open(no)` / `RecRef.Open(no, false)` (plain
  Open → `Known(false)`), and `RecRef.GetTable(SomeRec)` (inherits `SomeRec`'s resolved
  `tempState` from the routine's `record_variables`). CONSERVATIVE: derivation only fires
  when the routine has NO branching (`has_branching == false`) AND the call site is outside
  any loop (`loop_stack.is_empty()`). Anything uncertain (conditional, in-loop, unknown
  second arg for `Open`, unresolved source for `GetTable`) → `Unknown` (engine still fires;
  never wrongly `Known(true)`). OUT OF SCOPE by design: `Copy(..., ShareTable)` aliasing
  (cross-routine, speculative — documented non-goal). The pass is purely additive — it only
  sets temp on ops that were previously `Unknown`; the table-level and page-level overrides
  that run after it can still upgrade to `Known(true)` independently.

### Changed
- Vendored the rebaselined cli-a/cli-c goldens in-repo + restored the FROZEN al-sem
  archive (Task 16 / ts16 follow-up — the never-modify-al-sem rule). The cli-a html/json/
  terminal byte goldens and the cli-c cache fixtures had been regenerated in place inside the
  external (frozen) al-sem checkout; that violates the hard rule that al-sem is never modified.
  The 7 rebaselined files now live in-repo under `tests/cli-a-goldens/{html,json,terminal}/`
  and `tests/cli-c-goldens/cache/` (a self-contained 5-file fixture-cache + classification.json
  + dry-run.txt). The four harnesses (`cli_a_{json,terminal,html}_differential`,
  `cli_c_cache_differential`) gained a `resolve_golden`/local-dir resolver that prefers the
  in-repo override and falls back to the frozen al-sem path when no local override exists — so
  only the rebaselined fixtures read local; all ~unchanged cli-a goldens still read al-sem
  untouched. al-sem restored clean (0 modified files).
- Golden REBASELINE for the temp-state-tracking epoch + symbolReader cache bump 17→18
  (Task 16 / ts16). The temp-state epoch (Tasks 0–14) changed finding/projection CONTENT by
  design; the goldens are now Rust-OWNED baselines (the TS oracle is retired) and were
  REGENERATED from the current engine via a new env-gated (`REGEN_TEMP_GOLDENS`) regen path
  added to each differential harness (byte-parity suites write the engine output string;
  structural-JSON suites re-serialize the engine projection in the existing on-disk form).
  `KNOWN_DIVERGENCES.json` stays `[]` (divergences are NOT allowlisted — the diff was reviewed
  finding-by-finding). Suites moved: `r2a` L3 record-types (3 goldens — promoted object-global
  record vars now bind a tableId, `resolvedRecordVarTableIds` 228→232); `r2.5b-rt` cross-app
  (1 — `depBoundRecordVars` 2→6 from ABI/native dep-source promoted record vars); `r3a2`
  summary-core (11 — PD substitution flips inherited `tempState` parameter-dependent→known/
  unknown + `effectKey` tempfrag `p<i>`→`t`/`f`/`u`); `r3a3` cone-coverage (2 — `tempState`
  flips + `recordVariableId` now bound on previously-unbound ops); `r3a5` cross-app summary
  (1 — same flips + dep-routine `recordVariableId` bindings); `r3b` wrapped-parity (consumes the
  r3a5 golden); `r4` findings, `gate-sarif`, and `cli-a` html/json/terminal (the
  `ws-d1-multi-caller` d1 rootCause dropped "(temp state uncertain)" — now resolves physical via
  all callers; severity unchanged). The `cli-a-*` byte goldens + the `cli-c` cache fixtures were
  rebaselined and VENDORED in-repo (see the follow-up entry above) so the frozen al-sem archive
  stays unmodified. Relaxed the `r3a5_projection_is_byte_stable` `!contains("r0/")` sub-assertion (a
  too-strict heuristic the designed cross-app promotion legitimately invalidates — a promoted
  dep record var binds `recordVariableId: "r0/<hash>/rv/<name>"`, an internal id that
  canonically carries the `r0/` model-instance prefix); the determinism (a == b) and stable
  routine-id checks remain. The `symbolReader` cache version (`cache_prune.rs`) is bumped 17→18
  (the symbol-reader surface now carries promoted/ABI record vars with bound tableIds, so prior
  caches must invalidate); `cli_c_cache_differential` + its fixture cache updated to "18".
- d1 (`db-op-in-loop`) RV-1 CalcFields/FlowField gate (Task 11 / ts11 — the headline
  false-negative fix of the temp-state epoch). A `CalcFields`/`SetAutoCalcFields` on a
  record d1 resolved to TEMPORARY now downgrades to `info` ONLY when EVERY named field
  argument resolves (via the table model) to `field_class != "FlowField"` (a
  Blob/Normal field load on a temp record is genuinely in-memory). If ANY field arg is
  a FlowField — OR any field arg is UNRESOLVABLE (name not in the table, `table_id`
  None, table not indexed, or no capturable field args) — d1 KEEPS FIRING at normal
  severity with the honest note "(temporary record, but FlowField calculation queries
  the flow targets)". Rationale: a TEMPORARY record's FlowField is still computed by
  evaluating its CalcFormula against the (physical) flow-target tables — a real SQL
  round-trip, host tempness irrelevant. Previously the blanket temp downgrade wrongly
  suppressed temp FlowField CalcFields (a false negative). SOUNDNESS: the gate only
  ever PREVENTS a downgrade (keeps firing) when uncertain — it never newly suppresses a
  finding; the only behaviour change is temp FlowField CalcFields now fires (removes the
  false-negative). The CDO motivating case `Files.CalcFields("File Blob", …)` (Blob →
  in-memory) still downgrades correctly. Gate works for cross-app tables (`field_class`
  is modeled on both native `L3Field` and ABI `AbiField`).
- d1 (`db-op-in-loop`) now consumes the PATH-RESOLVED temp state instead of the
  terminal op's RAW `temp_state` (Task 10 / ts10, Component 3, RV-6 — the first real
  detector behaviour change of the temp-state epoch). For each finding, d1 calls
  `resolve_temp_along_path` over THAT finding's evidence path: resolved `Known(true)`
  → downgrade to `info` (existing suppression); resolved `Known(false)` → fires at
  normal severity with NO temp note (honest physical); resolved `Unknown` → "(temp
  state uncertain)" + normal severity (existing uncertain behaviour). A terminal op
  that is ALREADY `Known(_)`/`Unknown` (non-PD) resolves immediately with no stepping,
  so behaviour is UNCHANGED for it; only PD-terminal (by-var param) findings gain
  per-path precision — previously they fell to "(temp state uncertain)", now they
  resolve to a precise verdict per caller path.
- `resolve_temp_along_path` now enforces the L4 edge-kind ALLOWLIST (Task 10 / ts10,
  RV-6 soundness). It takes an `edge_kind_by_callsite` lookup (callsite id → resolved
  edge kind, derived from the combined graph d1 already holds) and, before stepping a
  hop, checks the kind is in `{direct, method, implicit-trigger}`; ANY other kind
  (`dynamic | interface | codeunit-run | report-run | page-run | event-dispatch`) or a
  callsite missing from the map STOPS the chase → `Unknown` (sound = fires). Without
  this guard a PD chased down a dynamic/interface/run hop with a `Known(true)`-sourced
  binding would resolve `Known(true)` where L4 returns `Unknown` — an unsound
  divergence that could SUPPRESS a real finding. Mirrors `substitute_pd_temp_state`.
- d1 merge-tie rule (Task 10 / ts10, RV-6). `merge_by_terminal` collapses every path
  sharing a terminal op into one finding; post path-resolution, two paths can DISAGREE
  on the temp-derived severity (caller-A path → info/temporary; caller-B path →
  normal/physical). The WORST severity now wins (deterministic, conservative — never
  let a temp path hide a physical path's finding) AND the temp note lists BOTH verdicts
  ("temp state varies by caller: physical via B; temporary via A", sorted). Reconciled
  before the merge so the canonical lift carries the worst severity + dual-verdict note.
- DESIGNED golden moves (deferred to Task 16 rebaseline): d1/r4 + downstream
  (cli-a json/html/terminal, gate SARIF) goldens move for multi-caller PD-terminal
  findings — temp-derived severity/note changes only (e.g. `ws-d1-multi-caller` drops
  its "(temp state uncertain)" note because all callers pass a physical record;
  severity unchanged). No non-PD finding moves; no non-temp severity changes.

### Added
- Shared per-PATH temp-state resolver `resolve_temp_along_path` (Task 9 / ts9,
  Component 3, RV-6) in `src/engine/l5/path_temp_resolve.rs`. A path-walker terminal
  db-op may carry `temp_state = ParameterDependent(i)` (depends on param `i` of the
  routine the op lives in); that symbolic index is only resolvable along a CONCRETE
  caller chain, so the SAME op reached from two different callers can resolve
  differently (per-finding truth: caller passing a temp local → `Known(true)`;
  caller passing a physical var → `Known(false)`). The helper starts from the
  terminal op's `TempStateKind`, then steps ONE frame toward the path ROOT per
  `ParameterDependent` level — using each hop's `callsite_id` to look up the parent
  routine's `argument_bindings` and applying the SAME substitution table as the L4
  per-callsite fold (`Some(Known(v))` → `Known(v)`; `Some(PD(j))` → `PD(j)` then chase
  `j` in the next frame up; `Some(Unknown)` / `None` / missing binding / missing
  callsite → `Unknown`). Still-PD at the path root (the op's tempness depends on an
  entry param with no caller in this path) → `Unknown`. The callee-param index RV-6
  asks the walker to expose per hop is DERIVED at resolve time from the L3 routine map
  (the same `ctx.routine_by_id` d1 builds) rather than added as a new serialized field
  — so NO walker/`EvidenceStep` struct changed and no R3a/trace/R4 golden moves.
  `WalkResult.path` orientation confirmed ROOT→TERMINAL. Sound by construction: only
  resolves to `Known(true)` when a concrete binding source on the path is itself
  `Known(true)`; all uncertainty → `Unknown` (fires). The helper is SHARED and not yet
  wired into any detector (d1 wiring is Task 10), so detector behaviour is unchanged.
- Param-source argument-binding resolution at the L4 PD substitution (Task 8 /
  ts8, RV-7 binding gap). When a caller FORWARDS its OWN record parameter as the
  argument (e.g. `procedure A(var Rec: Record X)` calls `Helper(Rec)`), the
  inherited effect's tempness depends on the CALLER's param, not a concrete var.
  A record-typed parameter is already present in the caller's L2
  `enclosing_record_variables`, so the forwarded-param arg's binding already
  carries `source_temp_state` = that caller param's own temp_state. The
  `substitute_pd_temp_state` PD arm (`summary_runner.rs`) now RE-SYMBOLIZES:
  `Some(ParameterDependent(j))` → `ParameterDependent(j)` (chaining the symbolic
  dependency UPWARD to the caller's own param index) instead of collapsing to
  `Unknown`. A forwarded `temporary`-keyword param still yields `Known(true)`,
  a by-value param `Known(false)`, and a genuinely-unknown / nameless source
  `Unknown`. Sound by construction: re-symbolizing PD→PD only PROPAGATES a
  symbolic dependency — it never invents `Known(true)`; a PD chasing itself
  around a recursive cycle stays PD (monotone) and the JACOBI fixed point
  converges because the effect_key includes the PD index, keeping the state
  space finite (verified: self-recursion + 2-cycle forwarding fixtures converge,
  no `MAX_FIXED_POINT_ITERATIONS` regression).
- Per-callsite substitution of `ParameterDependent` temp states at L4 effect
  inheritance (Task 7 / ts7, G5, RV-7) — when a caller folds in a callee
  `DbEffect` whose `temp_state` is `ParameterDependent(i)`, the CALLEE-frame index
  `i` (meaningless in the caller's frame) is now RESOLVED per-callsite through the
  caller's argument binding for callee param `i`, instead of being copied
  verbatim. In `summary_runner::compose_routine` the db-effects fold now branches
  on the callee effect's temp_state: a `ParameterDependent(i)` effect is rewritten
  via the new `substitute_pd_temp_state` helper and re-keyed with `effect_key_of`
  before insertion; non-PD (`Known`/`Unknown`) effects fold unchanged as before.
  Substitution table over `binding.source_temp_state`: `Some(Known(true))` →
  `Known(true)`, `Some(Known(false))` → `Known(false)`, `Some(Unknown)` /
  `Some(PD(_))` → `Unknown`, and `None` (the caller's-own-param-source / RV-7
  binding gap, resolved properly in Task 8) → `Unknown`. Event-dispatch edges (no
  `callsite_id`) and edge kinds with no modeled binding semantics
  (`interface | codeunit-run | report-run | page-run | dynamic`) → `Unknown`;
  only `direct | method | implicit-trigger` carry usable bindings.
  Sound by construction: substitution only NARROWS symbolic → binding-derived, all
  uncertainty becomes `Unknown` (fires), and `Known(true)` is produced ONLY from a
  binding source that is itself `Known(true)` — suppression stays gated on
  `Known(true)`. Re-keying naturally dedupes by `(op, tableId, operationId,
  tempfrag)`: identical substitution results merge while divergent "mixed caller"
  results stay DISTINCT (e.g. one caller passing a temporary local and one passing
  a physical local to the same callee op yield two distinct inherited effects,
  `Known(true)` and `Known(false)`). The per-op resolved-state space is finite, so
  the JACOBI fixed point stays bounded (no `MAX_FIXED_POINT_ITERATIONS` regression).

### Changed
- Scope-honest argument-binding `sourceKind` (Task 8 / ts8, RV-8). The L2 binding
  builder labels any non-parameter record-var arg `"local"` because object globals
  are only PROMOTED into a routine's `record_variables` later, at L3. After
  promotion runs (`l3_workspace.rs`), a binding whose source matches a PROMOTED
  GLOBAL record var (`scope == Some("global")`) is now RELABELED from `"local"` to
  `"global"`, removing the diagnostic mislabel. Only `"local"` bindings are
  eligible — `"parameter"` / `"implicit-rec"` / `"expression"` are untouched.
  Behavior-preserving: `d39`'s persistable-source allowlist now accepts `"global"`
  alongside `"local"` (a promoted global is a real caller var, persistable exactly
  like a local; the persist-after check matches by name regardless of scope), and
  `static_arg`'s named-source allowlist already accepted `"global"`. No detector's
  outcome changes for the global case.
- R3a-2 structural oracle `every_inherited_effect_traces_to_a_callee_effect` and
  the via-precedence oracle `merged_via_is_the_max_over_contributing_sources`
  (`tests/r3a2_oracles.rs`) now match inherited effects to their callee source via
  the substitution-aware `callee_key_sources_inherited` relation: a callee
  `parameter-dependent` effect (tempfrag `p<i>`) is a valid source for an inherited
  effect whose tempfrag was SUBSTITUTED (the invariant `op|tableId|operationId`
  prefix matches; only the tempfrag changed). Without this, Task 7's per-callsite
  re-keying would trip the old byte-equality invariant for PD-touching SCCs.

- ABI (dependency) temp capture + net-new per-param record-var temp-state modeling
  (Task 6 / ts6, G7, RV-4) — brings the cross-app `.app` symbol path to native+ABI
  shape parity so a detector behaves identically whether a record flows through a
  workspace routine or a dependency routine:
  - `parse_symbol_reference` (`symbol_reference.rs`) now READS the temp markers it
    previously ignored: `AbiParameter.is_temporary` from the param
    `TypeDefinition.Temporary == true`, and `AbiTable.is_temporary` from the
    table-level property `{"Name":"TableType","Value":"Temporary"}` (exact
    case-insensitive value match via the new `raw_table_is_temporary` helper —
    mirrors how `parse_field` reads `fieldclass`; NO string-sniffing). Verified
    against a real Continia Core 29.0 SymbolReference.json. (A return-type
    `Temporary` marker is intentionally not modeled — `AbiRoutine` has no return-temp
    slot and no consumer; documented in-source.)
  - The ABI projection (`projection.rs`) forwards the markers: `ProjectedParameter`
    gains `is_temporary`, `ProjectedTable` gains `is_temporary`, both populated in
    `project_abi_to_index`.
  - The ABI→L3 projection (`cross_app_l3.rs`) now SYNTHESIZES `record_variables` for
    record-typed parameters of dep routines (previously `record_variables: []`),
    each with a base `temp_state` per the native rule (mirroring
    `l2::scope::extract_record_variables`): `Temporary` marker → `Known(true)`;
    by-var record param WITHOUT marker → `ParameterDependent(param_index)`;
    by-value record param → `Known(false)`. Each var carries `is_parameter = true`,
    `parameter_index`, `scope = Some("parameter")`, and a `table_name` derived from
    the param `type_text` (`record_types::record_table_name_of`). `dep_table_to_l3`
    now forwards `is_temporary`, so the merged-whole `resolve()` runs the SAME
    table-level override (Task 4) — a param typed on a `TableType = Temporary` dep
    table resolves to `Known(true)`. ONE precedence rule everywhere; falls to the
    base temp_state (no override) when the type text yields no table name (engine
    never throws). Suppression-safe: `Known(true)` only from exact markers, every
    uncertain case stays `PD`/`Unknown`.
- Page `SourceTableTemporary = true` capture + implicit `Rec`/`xRec` `Known(true)`
  override (Task 5 / ts5, G4, RV-8):
  - `project_file` (`l3_workspace.rs`) now reads the `SourceTableTemporary` property
    for Page and PageExtension objects via `read_object_property`, setting
    `L3Object.source_table_temporary = Some(true)` on an exact case-insensitive match
    against `"true"` (trim + lowercase); `Some(false)` when present but not `"true"`;
    `None` when absent. Never `.contains()` / string-sniffing; engine never throws.
    `L3Object` is not serialised into any gate surface, so this never moves a golden.
  - Page-level override pass added to `resolve_routine_record_types` (`record_types.rs`),
    running after the table-level override: when the current object's
    `source_table_temporary == Some(true)`, every record op whose
    `record_variable_name` (lowercased) is `rec` or `xrec` is force-upgraded to
    `Known(true)`. Both `rec` AND `xrec` (RV-8: xRec alongside Rec). Purely ADDITIVE
    toward `Known(true)` — never downgrades; `SourceTableTemporary = true` is a
    structural page property that cannot be carried by physical-source pages, so the
    upgrade is sound (suppression-safe direction).
- Native `TableType = Temporary` capture + table-level override precedence
  (Task 4 / ts4, G3, RV-8):
  - `index_table` (`l3_workspace.rs`) now reads the object-level `TableType`
    property via `read_object_property` and sets `L3Table.is_temporary = true`
    on an EXACT case-insensitive match (trim + lowercase + `== "temporary"`;
    never `.contains()` / string-sniffing). A missing/other value → `false`
    (conservative). This is the only allowed temp signal — a structural property
    read. `L3Table` is not serialised into any gate surface, so this never moves
    a golden.
  - Final override pass in `resolve_routine_record_types` (`record_types.rs`),
    running AFTER all `table_id` resolution (declared vars, ops, lexical fallback,
    implicit Rec/xRec pass-3): for every record op whose resolved table is
    `is_temporary`, force `temp_state = Known(true)`, and likewise for the matching
    record VARIABLE. The "one precedence rule everywhere" — table-level temp WINS
    over keyword / no-keyword / by-value / by-var / `ParameterDependent(i)`. So a
    by-var PARAM of a temp table reports `Known(true)`, not the L2-stamped `PD(i)`
    (RV-8). Purely ADDITIVE toward `Known(true)`: only upgrades, never downgrades a
    `Known(true)` and never forces `Known(false)`. Table lookup uses the existing
    `SymbolTable::table_by_id`.
  - `TableView::is_temporary()` test-facing accessor.
- `extract_object_global_record_vars` in `scope.rs` (Task 2 / ts2, G1): captures
  the `temporary_keyword` on object-level `var_section` record variable declarations,
  producing `PRecordVariable` with `temp_state = Known(true/false)` and
  `scope = Some("global")`.  Non-record vars are skipped; `preproc_conditional_var_block`
  and dataitem-scoped var sections are conservative gaps (fall to Unknown, RV-8).
  Not yet wired into L3 projection (Task 3).
- Additive model fields for temp-state tracking epoch (Task 1 / ts1):
  - `PRecordVariable.scope: Option<String>` (`"local"` | `"parameter"` |
    `"global"`; `skip_serializing_if` keeps goldens stable; populated by later tasks).
  - `L3RecordVariable.scope: Option<String>` — forwarded from L2; field-allowlisted
    L3 projection never reaches goldens.
  - `L3Table.is_temporary: bool` (default `false`) — additive; L3Table is not
    serialised into any gate surface.
  - `L3Object.source_table_temporary: Option<bool>` (default `None`) — additive;
    L3Object is not serialised into any gate surface.
  - `AbiTable.is_temporary: bool` (default `false`) — slot for ABI temp capture
    (populated by Task 6).
  - `AbiParameter.is_temporary: bool` (default `false`) — slot for parameter
    `temporary` modifier (populated by Task 6).
  - `RawTypeDef.temporary: Option<bool>` (`#[serde(rename = "Temporary")]`) —
    deserialises the `Temporary` field from `SymbolReference.json`; consumed by
    Task 6.

### Fixed
- Object-global record vars are now promoted into EACH routine's
  `record_variables` during L3 assembly (Task 3 / ts3, G2), and member-var record
  operations re-derive their `temp_state` from the promoted set — the root-cause
  fix for the CDO false-critical class (a codeunit member
  `Files: Record "CDO File" temporary;` was never seen by the L2 body walk, so
  `Files.DeleteAll()` carried `tempState = Unknown`, fired a false critical, and
  d1 stamped "(temp state uncertain)"). Promotion honors AL shadowing: a routine's
  own param/local of the same name shadows the global (innermost wins). Shadowed
  globals are NOT promoted, keeping `record_variables` NAME-UNIQUE — which
  preserves the documented pass-1 `var_index_by_name` last-wins invariant in
  `record_types.rs` (a name-duplicated list would let the global clobber the
  local). The op `temp_state` backfill lives in `record_types.rs` pass-2a: when an
  op matches its declaring record var, `op.temp_state` is copied from that var
  (alongside the existing `table_id` / `record_variable_id` derivation).
- `record_types.rs` pass 2b `variable_decl_by_name` map changed from last-wins
  (unconditional `insert`) to first-wins (`entry().or_insert()`) so that a
  procedure-local declaration always shadows an object-global with the same name
  — the correct AL innermost-scope rule and a prerequisite for the tempState
  backfill epoch (RV-5).

## [0.7.0] - 2026-05-06

### Added
- Anonymous, opt-out failure-diagnostics telemetry (Azure App Insights).
  - Captures resolution misses, parser errors, indexer issues, and handler outcomes.
  - All AL identifier names hashed with a per-installation 32-byte salt that stays local.
  - Three disable mechanisms: `DO_NOT_TRACK=1`, `--no-telemetry`, `~/.al-call-hierarchy/config.json` `telemetry.enabled=false`.
  - Off by default in debug, test, and CI builds.
  - LSP request `al-call-hierarchy/telemetryStatus` for runtime introspection.
  - Schema documented in `docs/telemetry.md`.
  - Fire-and-forget export: `BatchSpanProcessor` on a dedicated tokio current-thread runtime; HTTP calls are non-blocking, individual export failures are silently dropped, and LSP request threads are never affected by network state. 10s/5s reqwest timeouts cap any single HTTP call; shutdown is bounded to a 3s budget.

## [0.5.0] - 2026-03-22

### Changed
- **BREAKING: Migrated to tree-sitter-al V2 grammar** — all tree-sitter queries and parsing logic updated for the rewritten grammar
  - `procedure name:` and `trigger_declaration name:` now hold `(identifier)`/`(quoted_identifier)` directly (no `(name)`/`(trigger_name)` wrapper nodes)
  - `member_expression` field renamed from `property:` to `member:`
  - `parameter` field renamed from `parameter_name:` to `name:`
  - Individual `*_property` nodes replaced by unified `property` node with `name:` and `value:` fields
  - `preproc_split_codeunit_declaration` renamed to `preproc_split_declaration`
- **tree-sitter-al is now a git submodule** instead of an external sibling directory — clone with `--recurse-submodules`
- `build.rs` defaults to `tree-sitter-al` (submodule) instead of `../tree-sitter-al`

### Removed
- `field_access` query pattern — merged into `member_expression` with `quoted_identifier` as member
- `named_trigger` / `onrun_trigger` handling — unified into `trigger_declaration`
- `extract_trigger_name()` helper — no longer needed with V2 grammar
- `property_display_name()` helper — replaced by reading `property_name` field directly

### Fixed
- EventSubscriber detection now correctly handles V2 attribute-as-sibling model (attributes are siblings of procedures, not children)

## [0.2.0] - 2025-02-03

### Added
- **Event Subscriber Integration**: Event subscribers are now shown in the call hierarchy
  - Parses `[EventSubscriber]` attributes to extract publisher object and event name
  - Event subscribers appear as "callers" in `incomingCalls` for the subscribed events
  - Shows `[EventSubscriber]` tag in the call hierarchy detail

- **Code Lens Support**: Reference counts and quality metrics displayed above procedures
  - Shows "N references | complexity: X, lines: Y, params: Z" lens above each procedure/trigger definition
  - Displays cyclomatic complexity, line count, and parameter count for each procedure
  - Highlights procedures with 0 references as potential dead code
  - Click to navigate to the references (via `al-call-hierarchy.showReferences` command)

- **Unused Procedure Detection**: Diagnostics for procedures with no callers
  - Publishes `HINT` severity diagnostics for unused procedures
  - Excludes triggers and event subscribers (they're called implicitly)
  - Tagged with `UNNECESSARY` for IDE-specific rendering (strikethrough, etc.)

- **Code Quality Diagnostics**: Warnings for potential code quality issues
  - High fan-in warning: procedures called by more than 20 other procedures
  - Long method warning: procedures spanning more than 50 lines
  - Diagnostics published at `INFORMATION` severity

- **External .app dependency support**: The server now resolves calls to procedures defined in compiled .app packages
  - Automatically parses `app.json` to discover declared dependencies
  - Finds matching .app files in the `.alpackages` folder with version matching
  - Extracts procedure definitions from `SymbolReference.json` inside .app files
  - Shows "(from AppName)" in call hierarchy for resolved external calls
  - Supports all standard BC object types: Codeunits, Tables, Pages, Reports, etc.

### Changed
- **Memory optimization**: `ExternalSource.app_version` now uses interned `Symbol` instead of `String`, reducing memory usage when loading large .app dependencies (~50-100MB savings for BC base apps)

### New capabilities
- `textDocument/codeLens` - Returns reference counts for all procedures in a file
- Diagnostics publishing via `textDocument/publishDiagnostics`

### New modules
- `app_package.rs` - Parser for .app files (ZIP with 40-byte NAVX header)
- `dependencies.rs` - Dependency discovery and resolution from app.json

### Dependencies
- Added `zip` crate for .app file extraction
- Added `roxmltree` crate for NavxManifest.xml parsing
