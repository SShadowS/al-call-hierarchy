# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
