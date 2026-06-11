# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed
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
