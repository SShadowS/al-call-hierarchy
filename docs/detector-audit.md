# Detector audit ledger (all 41 detectors, AL-semantics correctness)

Read-only audit of every detector (d1..d51) against AL semantics, hunting BOTH false positives
AND false negatives (the latter invisible to finding-triage). 10 agents. ~30 gaps across ~30
detectors. CLEAN: d17, d19, d46 (and largely d10, d13, d38). Grouped by shared root (fix once →
many detectors), then singletons, then priority.

## Shared-root class A — temp records not gated (FP) [HIGH volume]
The temp epoch gated d1/d3/d10/d33/d36/d37/d39/d40, but these still fire on `temporary` records:
- **d2** (event-fanout-in-loop): no temp guard; subscriber touching only a temp table → `any_db_subscriber=true` → fires.
- **d4** (repeated-lookup-in-loop): `d4.rs:57-82` no temp_state check → repeated lookups on temp vars fire.
- **d8** (commit-in-transaction): `writes_tables_of` counts TEMP-table writes toward the 3-table manager gate (FP inflation).
- **d29** (modify-in-subscriber): by-value/temp records flagged.
- **d43/d44/d45** (event capability cone): `capability_query::writes_tables_of` / `find_capabilities` carry NO `is_temp` annotation → temp writes produce spurious conflict/exposure findings.
ROOT: capability cone + these detectors lack the `op.temp_state Known(true)` / temp-table check. FIX: add the temp gate (and a temp annotation to the capability cone facts for d43-45).

## Shared-root class B — trigger-Rec suppression incomplete for TABLE-LEVEL triggers (FP) — **FIXED (commit `fix(engine-audit-b)`, engine branch)**
`is_platform_loaded_trigger_rec` (mod.rs ~:143) suppresses page triggers + table FIELD `OnValidate`,
but NOT table-level `OnInsert`/`OnModify`/`OnDelete`/`OnRename` — where the platform ALSO loads `Rec`
and AUTO-PERSISTS it after the trigger. Affects:
- **d21** (read-without-load): `Rec.TestField` in `OnModify` → FP.
- **d37** (validate-without-persist): `Rec.Validate` in OnInsert/OnModify → platform persists → FP.
- **d39** (record-left-dirty): no auto-persist-trigger gate → FP on normal trigger patterns.
FIX: add OnInsert/OnModify/OnDelete/OnRename (table-level, receiver Rec) to the platform-loaded/auto-persist gate; d37/d39 treat those triggers as persisting.
FIXED: `TABLE_TRIGGERS_REC_AUTO_PERSIST` added to the `is_platform_loaded_trigger_rec` table arm
(covers d11/d21/d37, which consume the gate directly); new `is_auto_persist_trigger_rec` gates d39's
caller-side persist check (`autoPersistTriggerRec` skip stat). Tests: `tests/gap_audit_b_table_triggers.rs`
(suppression + non-trigger / non-Rec controls); g9/g14 suites unchanged-green.

## Shared-root class C — loop-detector guards (Next-terminator + virtual table) missing in d2/d18 (FP)
- **d2**: `D2Policy.terminals_at` has NO `is_terminator_next` filter (d1 has it at d1.rs:616) AND no `op_targets_virtual_system_table` filter → `repeat..until Rec.Next()` + virtual-table reads in a subscriber fire falsely.
- **d18**: receives `_ctx` unused → no virtual-table gate; `SetRange` on `Date`/`Integer` virtual table in a loop fires with wrong advice.
FIX: apply the same d1 guards (terminator-Next, virtual-table) to d2; pass ctx + virtual-table gate to d18.

## Shared-root class D — RecordRef ops invisible as record_operations (FN, + one FP)
RecordRef `Insert`/`Modify`/`Find`/`FindSet` surface as CALL SITES, not `L3RecordOperation` entries:
- **d1** FN: a loop doing bulk DML via RecordRef is silent (terminals_at only scans record_operations).
- **d10** FN, **d36** FN, **d43/d44** FN: RecordRef writes/loads invisible.
- **d11** FP: `RecordRef.GetTable(Rec)` load is unrecognized → Modify after it fires.
ROOT: the op model does not capture RecordRef DML as record ops (deep — touches L2 capture). LARGER; scope carefully.

## Shared-root class E — first-match vs net-effect filter/load bugs (FP)
- **d41** (transitive-filter-loss) Gap-T [HIGH]: `d41.rs:111-120` uses `.find()` (first `SetRange`/`SetFilter`), ignoring an intervening `Reset` → fires when no filter was active. The EXACT net-effect bug fixed for d33 (G-3/G-17), inline in d41.
- **d33** FN-1: `was_filtered_before` scans slice order not source order; a `Reset`-before-`SetRange` in source but later in slice wrongly suppresses.
- **d42** Gap-W [HIGH]: `AddLoadFields` with no prior `SetLoadFields` treated as restricted load — but in BC `AddLoadFields` alone = FULL load → fires wrongly. Gap-Y: FlowField names in SetLoadFields flagged (BC ignores them).
FIX: net-effect (last SetRange/SetFilter/Reset by source position) for d41 + d33; d42 treat AddLoadFields-only as full load + skip FlowFields.

## Singleton high-value
- **d29 FP-1 [HIGH]**: `Modify(false)` / `Insert(false)` (RunTrigger=false — the canonical pattern to AVOID recursive trigger re-fire) is NOT exempted (d29.rs:117 no arg inspection). Also `ModifyAll` in `OnAfterModifyEvent` (raises OnAfterModifyAllEvent, no recursion).
- **d37 FN-1 [MED]**: `ModifyAll` in PERSIST_OPS (d37.rs:31) — but ModifyAll does NOT persist the CURRENT record's Validate result → Validate→ModifyAll passes the persist check while the change is discarded (silent FN).
- **d20 FN [MED]**: `unconditional_exit_kind` (body_walk.rs:202) misses `break_statement` → unreachable-after-`break` not flagged. (TS-parity gap.)
- **d22 FN [MED]**: implicit-trigger `Rec` FlowField reads invisible — `field_accesses` only recorded for declared `record_var_names`, not the implicit-Rec base frame (body_walk.rs:866) → `Rec."Balance (LCY)"` in OnAfterGetRecord generates no PFieldAccess → d22 silent.
- **d34 FN / d35 FN [MED, by-design]**: `Unknown` effect skipped (avoid-FP G6 choice) → transitive commit-in-loop / in-subscriber missed in partial-coverage. Contentious (open-world conservative) — likely DEFER.

## Singleton bugs (correctness)
- **d4 BUG-5**: finding id `d4/{routine}/{loop}/{varLc}` omits the literal key → two distinct keys in the same (routine,loop,var) → DUPLICATE ids (no merge_by_terminal).
- **d7 BUG-1**: rootCause doubles the anchor routine name (`chain.join` already includes it, appended again, d7.rs:218-223).
- **d7 FN-1**: SCC anchor = first SORTED member; if that is external/dep → `else continue` drops the whole cycle (should pick first WORKSPACE member).
- **d12 BUG-1**: missing `dep_routine_ids` primary gate (d12.rs:56-57 commented, not coded) → cross-app dep integration events fire d12.
- **d14 FP**: `internal_reachable_externally` hardcoded false (detector_context.rs:273) → `internalsVisibleTo` friend-only routines flagged dead. (Documented TODO — wiring app.json.)
- **d50 BUG**: `CommitBehavior::Suppress` not in the effective-commit set (d50.rs:139 has ignore/error only) → false fire.
- **d51 BUG**: HTTP GET before Error fires (io_direction not filtered, unlike the commit check, ordering_facts.rs:435-453); io_label fallback hardcodes "write-direction".
- **d47/d48/d51 FN**: IsolatedStorage.Set / TaskScheduler.CreateTask not modeled as IO (is_io_type narrows to HTTP|FILE, ordering_facts.rs:37).

## Priority fix order (clearest, highest-value first; group by shared root)
1. ~~**B** — table-level trigger-Rec gate (d21/d37/d39) — extend the existing mod.rs gate, high-volume, easy.~~ DONE.
2. **A** — temp-gate d2/d4/d8/d29/d43/d44/d45 — extend temp check / cone temp annotation, high-volume.
3. **C** — d2 + d18 loop guards (terminator-Next, virtual table) — mirror d1, easy.
4. **E** — d41 net-effect filter + d42 AddLoadFields-full-load + FlowField — clear FP bugs.
5. **d29 Modify(false)** exemption — clear, canonical pattern.
6. **bugs**: d4 id, d7 rootCause+anchor, d12 dep gate, d50 Suppress, d51 io_direction — small correctness fixes.
7. **FN**: d20 break, d22 implicit-Rec FlowField, d37 ModifyAll-not-persist — premise-narrowing fixes.
8. **D** (RecordRef-as-op) — larger (L2 capture); **d34/d35 Unknown-skip**, **d14 internalsVisibleTo**, **d47/d48/d51 IsolatedStorage/TaskScheduler** — investigate/defer (open-world or schema).
