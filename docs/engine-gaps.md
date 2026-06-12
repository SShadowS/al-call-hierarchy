# Engine detector-quality gaps (backlog)

False-positive / quality classes surfaced by triaging the **post-temp-state-epoch** binary
against a real shipping app (Continia DocumentOutput / Cloud, ~2260 findings; 441 crit+high
in primary scope reviewed against source → 93 false positives, ~21%). The temp-state epoch
fixed the *declared*-temp class (the `ClearFiles` d33 critical is gone); these are the
classes that remain. Ordered by volume × ease.

Each gap: symptom, evidence (which triage batches + example sites), suggested approach.
This is a backlog for later work, not a committed plan.

> **Consolidated golden rebaseline DONE** (`chore(engine-gaps): rebaseline goldens
> after G-1..G-12 detector-gap fixes`). The per-gap "rebaseline deferred / remains
> with the consolidated rebaseline task" notes below are now SETTLED. Two content
> classes actually moved goldens: (a) **G-4** d1 transitive `rootCause` text on
> `ws-d1` (r4) + `ws-d1-multi-caller` (r4 / cli-a json+html+terminal / gate-sarif) —
> field-level diff: only `rootCause`, fingerprints/severity/ids unchanged; (b)
> **G-12** d3 PK-only existence-check `Get` suppression on `ws-inline-suppress`,
> whose anti-degenerate witness was restored by editing the fixture so
> `UnsuppressedD3` reads a Normal field (`Name`) after the `Get` (gate-suppress
> SARIF/PR-summary + the `ws-inline-suppress` L2 feature golden rebaselined). All
> other gap fixes moved no in-repo golden. `cli_c_cache` did not move → no cache
> version bump. `KNOWN_DIVERGENCES.json` stays `[]`.

---

## G-1 — d1 fires on the `Next()` loop terminator (highest volume, easiest)

**Status: FIXED (commit `fix(engine-g1): suppress d1 on the loop's own Next() terminator (G-1)`).**
Structural signal: the L2 body walk marks a record op that sits inside the `condition` field
of its NEAREST enclosing `repeat_statement` (`PRecordOperation.in_until_condition`,
serde-skipped → feature goldens byte-identical; forwarded through `L3RecordOperation`). d1
skips `op == "Next" && in_until_condition` in BOTH the direct in-loop branch and
`terminals_at` (the callee's own terminator no longer fires transitively from an ancestor
loop). A mid-body `Next` on a different cursor, any non-Next db op, and the in-outer-loop
cursor opener all keep firing. Tests: `tests/gap_g1_next_terminator.rs`. No in-repo golden
moved (the pre-loop cursor-opener heuristic already kept the simple direct shape out of every
fixture golden; no fixture exercises the transitive/nested-opener shapes); the CDO-run
rebaseline remains with the consolidated rebaseline task.

**Symptom:** `d1-db-op-in-loop` flags `Rec.Next()` when it is the `until Rec.Next() = 0`
terminator of the very loop being iterated. That `Next()` *is* the loop's advancement, not
an extra DB op added to the body — removing it breaks the loop.

**Evidence:** the single largest FP class. Batch 5 (lines 191, 966, 972), batch 6 (CDO Setup
Data 268/291/314, CDO Send Cust. Statement Mgt. 245/273, CDO Handled Fields 74/105/238),
batch 7 (multiple `Next` on RecordRef terminators), batch 10 (VendorLedgerEntry.Next:354,
EMailLog.Next:189). ~15+ FPs.

**Approach:** in the d1 candidate filter, exclude a `Next()` op whose call site is the
`until`-condition position of the *enclosing* `repeat` (the loop it advances), as opposed to
a `Next()` in the body that advances a *different* cursor. The loop/op structures already
carry the loop stack; the terminator is the `Next()` on the same record var as the loop's
iterator at the loop's `until`. Net: removes the biggest FP class with a structural check.

## G-2 — runtime-implied tempness not inferred (high volume, harder)

**Status: FIXED (commit `fix(engine-g2): infer tempness from IsTemporary-Error guards (table contract + routine entry guard) (G-2)`).**
Both sub-features land as exact structural matches of the
`if not <recv>.IsTemporary[()] then Error(...)` guard (`is_temporary_error_guard` in
`src/engine/l3/l3_workspace.rs`: if with NO else, condition `not <recv>.IsTemporary[()]`
or `<recv>.IsTemporary[()] = false`, bare-identifier receiver, zero-arg IsTemporary,
then-branch an `Error(...)` call or a `begin Error(...); end` block with exactly that
statement). (1) Table contract: `index_table` marks `L3Table.is_temporary` when any
OnInsert/OnModify/OnDelete/OnRename trigger contains the guard at TOP level (receiver
`Rec`) — the existing table-level override then upgrades all ops to `Known(true)`.
(2) Routine entry guard: when the guard is the routine's FIRST executable statement and
the receiver is a record var/param (incl. promoted globals) or implicit `Rec`/`xRec`,
`L3Routine.entry_temp_guard_receiver` is set at L3 assembly and a new override pass in
`record_types.rs` upgrades that receiver's ops + record var to `Known(true)`. Any
deviation (guard not first, nested table guard, non-negated condition, exit-not-Error)
→ untouched → fires. Tests: `tests/gap_g2_runtime_temp.rs`. No in-repo golden moved by
this change (no fixture contains an IsTemporary guard); the CDO-run rebaseline remains
with the consolidated rebaseline task.

**Symptom:** records that are temporary *by contract at runtime* but not by structural
declaration stay `Unknown` → detectors fire. Two shapes:
- a **table** with `OnInsert` (or other trigger) doing `if not Rec.IsTemporary then Error(...)`
  — every instance is provably temp (`CDO File`, Table 6175301).
- a **routine** opening with `if not Rec.IsTemporary then Error(...)` — its body operates on a
  temp record (`CDO Printer::UpdateFromXml`, `CDO Continia Online PDF Mgt::EmbedFiles`,
  `CDO E-Mail Template Line::FindAndInsertSignaturesInTemplate`).

**Evidence:** batch 1 (CDO File ops, 8 FPs), batch 9 (EmbedFiles, signatures, ~3), batch 11
(CDO File SetAttachmentsCopy, CDO Printer UpdateFromXml, 2). ~15 FPs.

**Approach:** two sub-features, both structural reads (no dataflow):
1. **Self-guarding temp table:** at table indexing, if a trigger body contains
   `if not <Rec>.IsTemporary then Error` (or `IsTemporary()` guard → Error), mark the table
   `is_temporary`-by-contract → ops on it resolve `Known(true)` like `TableType=Temporary`.
2. **Entry-guard temp routine:** if a routine's first statement(s) include
   `if not <param/Rec>.IsTemporary then Error`, treat that record as `Known(true)` within the
   routine. This is the runtime-guard analog of the structural capture the epoch added.
   Suppression-direction safe: the guard *proves* tempness, so `Known(true)` is sound.

Note the conservative current behavior is acceptable (fires rather than suppresses), but this
is the dominant *temp*-related FP source after the epoch.

## G-3 — d33 blind to filters set via helper procedures (interprocedural)

**Status: FIXED (commit `fix(engine-g3): d33 recognizes filters set via one-hop helper
procedures (G-3)`).**
New gate `record_filtered_by_call_before` (`src/engine/l5/detectors/mod.rs`), consulted by
d33 after its intraprocedural `was_filtered_before` scan — the filter analog of G-10's load
gate, sharing the same one-hop callee-summary join (extracted into
`callee_applies_op_to_by_var_arg`: resolved call edge + resolved by-`var` binding via
`resolved_call_edge_by_callsite` / `upgraded_bindings_by_callsite`, then a predicate over
the callee's `record_operations` on the bound parameter). The predicate requires the
callee's NET effect on the parameter to be filtered: its last `SetRange`/`SetFilter`/`Reset`
op (by source position) on that parameter is a filter (`RECORD_FILTER_OPS`, the exact set
d33 uses intraprocedurally, now shared), not a `Reset`. A caller-side `Reset` between the
helper call and the bulk op voids that call site. One hop only; suppression-direction safe:
no filter call, non-filtering callee, by-value binding, unresolved callee, filter call
after the bulk write, callee filter-then-Reset, and caller-side Reset all keep firing.
Tests: `tests/gap_g3_interproc_filter.rs` (helper-SetRange/SetFilter suppressions + six
controls). No in-repo golden moved; the CDO-run rebaseline remains with the consolidated
rebaseline task.

**Symptom:** `d33-unfiltered-bulk-write` fires on `DeleteAll`/`ModifyAll` when the filter was
applied by a *helper procedure call* (`SetTemplateFilter(Rec)`, `GetAttachments(Rec)`,
`SetMergeFieldFilter(Rec)`) immediately before, rather than an inline `SetRange`/`SetFilter`.

**Evidence:** batch 9 (CDO E-Mail Template Line OnDelete 456/458, DeleteEMailTemplate 872),
batch 10 (Output Profile Conflicts, Payment Link Template UpdateFromXml, MergeField OnDelete).
~5 FPs.

**Approach:** when a `DeleteAll`/`ModifyAll` receiver has, earlier in the routine, a call to a
procedure that takes the receiver `var` and whose body sets a filter on it, treat the receiver
as filtered. Needs a shallow interprocedural "does this callee set a filter on the by-var arg"
summary (the call-graph + record-op model already exist). Scope to one hop to start.

## G-4 — transitive loop over-attribution (d1)

**Status: FIXED (wording) (commit `fix(engine-g4): clarify transitive db-op-in-loop rootCause wording (G-4)`).**
NOT suppression — investigation confirmed these findings are mostly REAL: the terminal
op runs once per ANCESTOR iteration, which is exactly a db-op-in-a-loop's SQL cost. The
problem was attribution clarity: the rootCause (`"A loop in X reaches <Op> on <Table>."`)
never named the terminal routine Z, while the finding's primaryLocation points INTO Z —
so the text read as if Z itself looped. Fix (in `build_finding`, d1.rs): when the
terminal routine differs from the loop routine AND the terminal op's own `loop_stack` is
empty (the EXACT structural signal that the loop is purely ancestral), the rootCause
becomes `"A loop in X reaches <Op> on <Table> in Z, which has no loop of its own — the
operation runs once per iteration of that loop."` (mirrors d48's terminal-naming
precedent). Direct in-loop ops and transitive terminals inside the CALLEE's own loop
keep the original wording byte-identical. Presence/severity/confidence/ids/
rootCauseKeys/fingerprints all unchanged (fingerprint hashes rootCauseKey, not text);
the merge-tie temp-note strip/insert still lands on the `[tempNote][setupNote].` tail in
both shapes. The optional confidence lowering (likely→possible) was SKIPPED — wording
alone resolves the triage confusion without touching the confidence model. Tests:
`tests/gap_g4_transitive_wording.rs`. Moves d1 rootCause TEXT in `ws-d1` /
`ws-d1-multi-caller` r4/cli-a/gate-sarif goldens (field-level diff: only `rootCause`);
rebaseline deferred to the consolidated gap-fix rebaseline task.

**Symptom:** the "a loop in X reaches <op> in Z" framing fires on an `Insert`/op in a routine
Z that has **no loop of its own**, because an ancestor X loops. The op runs once per ancestor
iteration, but the finding reads as if Z loops — and some of these are genuinely not the
intended target (single op in a non-looping leaf).

**Evidence:** batch 7 (CDO Log Management CreateEDocLogEntries:149, CreateEDocLogErrorEntries:189
— single Inserts in non-looping routines), batch 10 (Electronic Document Mgt CreateSalesSourceDocs:171).

**Approach:** review the transitive-loop attribution: distinguish "op is inside a loop somewhere
on the path" (real, keep) from "op is a single statement in a leaf, the loop is purely in an
ancestor and the op is invoked once per iteration intentionally" (often the intended design).
At minimum, lower confidence / sharpen the rootCause wording when Z itself has no loop.

## G-5 — wrong table name in rootCause (call-graph symbol resolution)

**Status: FIXED (commit `fix(engine-g5): correct op→record-var→table name binding in multi-subloop routines (G-5)`).**
NOT a sub-loop binding bug — the op→record-var→table-name binding (L2 receiver capture,
`record_types.rs` name-keyed passes, `describe_table` tiers) was correct all along (locked
by regression guards). The actual root cause is a TABLE-ID COLLISION: a `tableextension`
declaration is indexed as an `L3Table` stub whose internal id reuses the EXTENSION's own
object number (`${appGuid}/table/${extNumber}`, kept so `merge_extension_fields` can find
the extension's fields). When a real table in the same app shares that number, the stub
clobbered it in every LAST-wins `table_by_id` lookup, so `describe_table` tier 1 rendered
the EXTENSION's name (`CDOReturnShipmentHeader` / `CDOPurchaseReceiptHeader` / `CDOJobExt`
are tableextension names whose numbers collide with the real tables the ops target). The
sequential-sub-loop correlation in the evidence was coincidental. Fix: new
`L3Table::is_extension_stub` marker + REAL-over-stub collision preference in every table
lookup map (`SymbolTable` by-name/by-id, `table_by_id_preferring_real` →
`DetectorContext`, HTML formatter, policy engine); LAST-wins preserved within the same
kind; `merge_extension_fields` untouched (lockstep with the projected twin). Name-only:
finding presence/severity/ids/fingerprints unchanged. Tests:
`tests/gap_g5_wrong_table_name.rs` (collision repro in both assembly orders + sequential /
unloaded-type / transitive multi-subloop guards). No in-repo golden moved; the CDO-run
rebaseline remains with the consolidated rebaseline task.

**Symptom:** rootCause text names the wrong table/record for some findings — e.g. reports
`CDOReturnShipmentHeader` / `CDOPurchaseReceiptHeader` / `CDOJobExt` where the source op is on
`MergeTableTopBottom` / `HtmlTableStyle` / `HtmlTableStyleLine` local vars. The finding is
otherwise real; only the name is wrong. Erodes trust in every finding.

**Evidence:** batch 2 (ExportAsXML 387/391/395/399, ImportXmlDocument 866/877), batch 3
(CreateMergeTables sub-loops 558/559/569/570/580/581). Same routines with several sequential
sub-loops over different local record vars.

**Approach:** the name resolution appears to mis-bind the record var for later sub-loops in a
routine that has multiple sequential `repeat` blocks over different vars (picks an earlier or
unrelated symbol). Audit the op→record-var→table name binding when a routine has multiple
record vars / sub-loops; likely a "first var wins" or stale-binding bug in the projection.

## G-6 — BC system/virtual tables flagged as DB ops

**Status: FIXED (commit `fix(engine-g6): skip SQL-cost detectors on BC virtual/system tables (G-6)`).**
Shared exact-name gate in `src/engine/l5/detectors/mod.rs` (same pattern as G-9):
`VIRTUAL_SYSTEM_TABLES` allowlist (`AllObj`, `AllObjWithCaption`, `Field`, `Key`, `Object`,
`Object Metadata`, `Table Metadata`, `Page Metadata`, `Codeunit Metadata`, `Report Metadata`,
`Database Locks`, `Session`, `Active Session`, `Integer`, `Date`) +
`op_targets_virtual_system_table`: the op's type did NOT resolve to a workspace table (a
user table with a colliding name stays physical → keeps firing) AND the record variable's
DECLARED type name matches the allowlist exactly (case-insensitive). Consulted by d1 (direct
in-loop branch, `virtualTable` skip stat, AND `terminals_at` so virtual ops never fire
transitively) and d4 (candidate filter). d3/d33 need no gate — they already bail on
unresolved-table ops, and a virtual table never resolves in the source-only workspace.
Anything off the allowlist keeps firing. Tests: `tests/gap_g6_virtual_tables.rs`
(suppression direct/transitive/d4 + loaded-physical and unloaded-non-virtual controls). No
in-repo golden moved (no fixture performs record ops on a virtual table); the CDO-run
rebaseline remains with the consolidated rebaseline task.

**Symptom:** `d1` fires on reads of BC system virtual tables (`AllObjWithCaption`, `Field`)
which have no physical SQL backing (they read BC's internal metadata store). Engine marks them
"type not loaded" and fires conservatively.

**Evidence:** batch 5 (AllObjWithCaption 223/233/239, Field 666/667/677). 6 FPs.

**Approach:** maintain a small allowlist of known virtual/system tables (AllObjWithCaption,
Field, Object, Integer, Date, etc.) classified non-physical → SQL-cost detectors skip them.
Cheap, eliminates a clean FP class.

## G-7 — dead code via commented-out call site not detected

**Status: FIXED (down-confidence) (commit `fix(engine-g7): down-confidence perf findings in provably-dead routines (G-7)`).**
d14's dead-determination is factored into the reusable `provably_dead_routine_ids`
(`src/engine/l5/detectors/d14.rs` — the SAME `classify_routine` criteria d14 emits from:
forward-BFS unreachable from the entry-point closure + `local`/app-scoped-`internal` access +
not a Test object + not a property-expression host + not itself a root). d1 consults it
post-merge: when EVERY path root routine (canonical + additionalPaths) is provably dead, the
finding KEEPS FIRING at the same severity but its confidence drops one notch
(likely → possible) and the rootCause gains "(looping routine appears unreachable from any
entry point; see d14-dead-routine)". Deliberately NOT suppression (d14 itself has open-world
FPs — compounding them would hide real findings); any live or merely-unprovable path root
keeps full confidence. d1 only for now (the gap's evidence is d1-only; other detectors can
adopt the same helper if triage shows volume). Covered by `tests/gap_g7_dead_routine.rs`.

**Symptom:** `d1`/`d14` fire on a routine whose only caller is commented out — the routine is
dead, the finding moot.

**Evidence:** batch 4 (CDO Data Upgrade `UpgradeOutputProfileOnDocsWorker`:115; its caller
`UpgradeOutputProfileOnDocs` is commented out at line 17).

**Approach:** the parser already drops commented code, so the call edge is correctly absent —
the gap is that reachability/dead-routine analysis didn't down-rank the finding. Cross-detector:
when d14 marks a routine dead, other detectors' findings rooted only in that routine could be
suppressed or down-confidenced. Low volume; lowest priority.

## G-8 — residual global / by-var-param temp resolution gaps

**Status: FIXED (commit `fix(engine-g8): backfill promoted-global temp state into call-arg
bindings — forwarded codeunit-global temp records resolved "uncertain"`).**
Investigation ground truth (all regression-guarded in `tests/gap_g8_residual_temp.rs`):

- **Direct ops on a codeunit-global `temporary` record** (`TempErrors.Insert()` /
  `FindSet()` / `Next()` in the global's own object, table NOT in the workspace): already
  `Known(true)` via Task-3 promotion + pass-2a rebind — NOT a bug.
- **Keyword-temp by-var param** (`GetUpgradeData(var Temp: Record X temporary)`):
  already `Known(true)` by contract-trust (Task 8 / RV-3), caller irrelevant — NOT a bug
  (the batch-7 case is covered IF the params carry the keyword; without it, see next).
- **Keyword-LESS by-var param, d1 DIRECT finding** (loop + op in the same routine): PD at
  the path root → `Unknown` → "uncertain" is CORRECT per-path conservatism (a caller-side
  temp local only binds the TRANSITIVE finding rooted at that caller) — out-of-model by
  design, documented in the regression guard.
- **THE REAL RESIDUAL BUG — temp global FORWARDED by-var into a helper** (the
  eDocuments-Dispatcher shape: `LogError(TempErrors)` in a loop, op inside the helper's
  keyword-less by-var param): the L2 binding builder only matches the routine's OWN
  params/locals, so the global arg's binding was `sourceKind:"unknown"` with NO
  `sourceTempState`; L4 `substitute_pd_temp_state` and L5 `resolve_temp_along_path` both
  collapse a missing source to `Unknown` → "uncertain". Fixed in
  `src/engine/l3/l3_workspace.rs` (the RV-8 relabel block, post-promotion): backfill the
  binding from the promoted-global record var — bare-identifier arg text only, an
  innermost-declaration-must-be-the-global shadowing guard, temp state copied from the
  global's exact `temporary`-keyword signal (suppression-direction discipline). Non-temp
  globals backfill Known(false) and keep firing. No in-repo golden moved (no golden
  fixture forwards an object-global record var by-var).

**Symptom:** a few module-level `temporary` globals and by-var temp params still resolve
"uncertain" after the epoch. The epoch promotes object-global record vars and substitutes PD,
so these should mostly be covered — worth confirming whether the remaining cases are a real
residual gap or an out-of-model shape.

**Evidence:** batch 7 (CDO Aut. Statement Upgrade Mgt `GetUpgradeData` Temp* by-var params from
Page 6175450 caller), batch 9 (CDO eDocuments Dispatcher `TempErrors: Record "Error Message"
temporary` codeunit global). 

**Approach:** verify against the temp-state model: do these go through the object-global
promotion path (G2/Task-3) and PD substitution (Task-7/8)? If `TempErrors` is a codeunit global
declared `temporary` and still uncertain, that's a promotion miss to investigate; if it's a
by-var param resolved per-path where one path is genuinely unbound, the Unknown is correct.
Confirm before treating as a bug.

---

# Additional gaps from the medium/low triage

Triaging the 524 medium+low primary findings (→ 329 real, ~195 FP, ~37%) surfaced these,
dominated by detectors that reason about *load state* (d3/d11/d21/d37) rather than loops.

## G-9 — page/table trigger `Rec` is platform-loaded (NEW, the biggest medium/low FP class)

**Status: FIXED (commit `fix(engine-g9): suppress d11/d21/d37 on Rec in page/table triggers (platform-loaded)`).**
Shared structural gate `is_platform_loaded_trigger_rec` (`src/engine/l5/detectors/mod.rs`):
`routine.kind == "trigger"` AND (Page/PageExtension trigger named `OnValidate`/`OnAction`/
`OnAfterGetRecord`/`OnDrillDown`/`OnAfterGetCurrRecord`, OR Table/TableExtension `OnValidate`
— always a field trigger) AND the op's receiver is `Rec` (case-insensitive). Wired into
d11/d21/d37 as a skip (`triggerRec` stat). Anything uncertain keeps firing. Tests:
`tests/gap_g9_trigger_rec.rs` (suppression + non-trigger / non-Rec controls). No in-repo
golden moved (no fixture exercises trigger-Rec for these detectors); CDO-run rebaseline
remains with the consolidated rebaseline task.

**Symptom:** `d11-modify-without-get`, `d21-read-without-load`, and `d37-validate-without-persist`
fire on `Rec` inside **page triggers** (`OnValidate`, `OnAction`, `OnAfterGetRecord`,
`OnDrillDown`, `OnAfterGetCurrRecord`) and **table field `OnValidate`** triggers. In all of
these the AL platform has already loaded `Rec` before the trigger runs, and a field
`OnValidate` calling `Validate(...)` on a sibling field is normal field-chain validation whose
persistence is the caller's responsibility — not a missing Get / lost Validate.

**Evidence:** the single largest medium/low FP source — ~40+ FPs across batches 2, 5, 6, 9, 10,
11, 12 (e.g. CDO E-Mail Template Card page triggers, CDO Setup/CDO Vendor OnValidate, every
SMTP Setup OnAction).

**Approach:** when the enclosing routine is a page trigger OR a table field/record trigger and
the subject record is `Rec` (the trigger's implicit current record), suppress d11/d21/d37 — the
platform guarantees `Rec` is loaded, and field-chain `Validate` is not a persist obligation of
the trigger. Structural (trigger kind + receiver == `Rec`), suppression-direction safe.

## G-10 — record-loading wrappers not recognized as loads (NEW)

**Status: FIXED (commit `fix(engine-g10): recognize GetBySystemId + one-hop load-wrappers
as record loads (G-10)`).**
Shared gate `record_loaded_by_call_before` (`src/engine/l5/detectors/mod.rs`), consulted
by d11/d21 after their intraprocedural `loaded_before` scan. Tier 1 (platform built-ins):
a member CALL SITE `<var>.GetBySystemId(...)` strictly before the op counts as a load —
exact-name allowlist `PLATFORM_LOADER_METHODS`, receiver must equal the record variable
(case-insensitive). Tier 2 (one-hop callee summary): the record was passed as an argument
whose binding RESOLVED to a by-`var` record parameter of a workspace callee
(`resolved_call_edge_by_callsite` + `upgraded_bindings_by_callsite`, the d37/d39/d40 join)
and the callee's own body performs a recognized load op on that parameter —
`RECORD_LOAD_OPS` is the exact set d11/d21 use intraprocedurally, now shared so the two
stay in lockstep. Covers custom `FindXxx`/`GetXxx` wrappers, `InsertIfNotExists` (Insert
is a recognized load) and var-out facade loaders in one mechanism; this is the load analog
of G-3's filter summary (one hop, callee body only — G-3 can reuse the pattern). Anything
uncertain (unresolved callee, by-value binding, non-loading callee, call after the op,
different variable, cross-app context without the resolved-edge index) keeps firing.
Tests: `tests/gap_g10_load_wrappers.rs` (both suppressions + six controls). No in-repo
golden moved; the CDO-run rebaseline remains with the consolidated rebaseline task.

**Symptom:** `d11`/`d21` fire even though the record was loaded by a method that isn't the
literal `Get`/`Find`: `GetBySystemId`, custom `FindTemplate`/`FindXxx` wrappers,
`InsertIfNotExists`, and `var`-out facade loaders like
`GetApplicationAreaSetupRecFromCompany(var Rec, …): Boolean`.

**Evidence:** batches 1 (GetBySystemId ×4), 10 (FindTemplate ×4, SetDataRecord/GetDataRecord),
11 (facade loaders, InsertIfNotExists), 12.

**Approach:** treat platform loaders `GetBySystemId`/`GetBySystemId` and any callee that takes
the record `var` and performs a `Get`/`Find` on it (one-hop summary, same machinery as G-3) as
satisfying the "record is loaded" precondition. Custom `FindXxx` wrappers need the one-hop
callee summary; the built-ins (`GetBySystemId`) are a cheap allowlist.

## G-11 — d20 misfires on trailing comments / single-line & conditional exits (NEW, parser bug)

**Status: FIXED (commit `fix(engine-g11): d20 only fires on unconditional exit with a real following statement (G-11)`).**
Root cause was a single bug: the L2 unreachable-after-exit scan (`src/engine/l2/body_walk.rs`,
code_block entry) treated every `named_children` of a `code_block` as a statement, and in the
V2 grammar `comment` / `multiline_comment` / `pragma` nodes ARE named children — so a trailing
or own-line comment after an `exit` was flagged as the "next statement". The scan now filters
that trivia out. The single-line-body and conditional-exit shapes were already structurally
correct in the Rust engine (a lone `exit(expr)` has no following sibling; an
`if … then exit(x)` sibling is an `if_statement`, never classified unconditional) — the
triaged FPs there were the comment-trailed variants of those shapes; both are locked in by
tests. Suppression-direction safe: a real statement after an unconditional exit still fires,
even with a comment between. Tests: `tests/gap_g11_d20_position.rs`. No in-repo golden moved
(full `cargo test` green); the CDO-run rebaseline remains with the consolidated rebaseline
task.

**Symptom:** `d20-unreachable-after-exit` flags as "unreachable" (a) a trailing inline comment
on an `exit(...)` line (`exit(0); // note` → the comment column is treated as a statement), (b)
a single-line function body that is just `exit(expr)`, and (c) the fall-through `exit(0)` after
a *conditional* `if … then exit(x)`.

**Evidence:** batches 4 (ReadSendEDocsOrdinal conditional exit), 7 (conditional exit-in-loop),
11 (CDO Functions single-line exits ×2), 12 (trailing-comment exits ×2). ~6 FPs, all clearly
wrong.

**Approach:** fix d20's position/reachability logic: an `exit` only makes following code dead if
it is UNCONDITIONAL and there is an actual subsequent statement (not a comment, not end-of-body).
The current logic appears to use the column past the `;` (catching comment text) and to ignore
the `if`-guard. Pure d20 correctness fix; suppression-direction safe (removes false fires).

## G-12 — d3 over-fires on PK-only / FlowField / existence-check Gets (NEW refinements)

**Status: FIXED (commit `fix(engine-g12): d3 excludes PK/FlowField/existence + pre-Get SetLoadFields (G-12)`).**
In d3, the "unloaded fields accessed" set now excludes the table's first-key (PK) field names
and `field_class == "FlowField"` fields (exact structural signals; an unresolved field name
stays in the set — keep firing); an existence-check Get whose post-Get accesses are all
PK/FlowField (or nothing) leaves the set empty → no witness, no emit. The pre-Get
`SetLoadFields` data-flow gap was QUOTED arguments: `deriveLoadStates` already walks all ops
in source order, but the L2 body walk keeps the raw `"Unit Price"` argument text while field
accesses are recorded unquoted — the load set is now quote-normalized so a quoted pre-Get
`SetLoadFields` covers the later access. Controls locked in: PK+normal and FlowField+normal
mixed reads still fire (missing list names the normal field only), an uncovered normal read
and an incomplete pre-Get SetLoadFields still fire. `tests/gap_g12_d3_refinements.rs`.

**Symptom:** `d3-missing-setloadfields` fires when (a) the only field read after a `Get` is the
primary key (always loaded regardless of SetLoadFields), (b) the accessed field is a **FlowField**
(needs `CalcFields`, not `SetLoadFields`), or (c) the `Get` is an existence check followed by an
unconditional `exit` with no field read. Also: d3 misses a `SetLoadFields` that was set *before*
the `Get` or via an assignment the engine's data-flow didn't track.

**Evidence:** batches 1, 8 (SetLoadFields set before Get missed; Get-as-existence-check), 10/12
(PK-only Gets, FlowField fields).

**Approach:** in d3, (1) exclude PK fields from the "unloaded fields accessed" set (always
available), (2) exclude FlowField fields (orthogonal — that's d22's domain), (3) suppress when
the post-Get path reads no field, and (4) tighten the SetLoadFields data-flow to catch a
SetLoadFields anywhere before the Get in the routine. Each removes a clean FP sub-class.

---

## Suggested order

1. **G-9** (trigger `Rec` loaded) — biggest medium/low FP class, structural, suppression-safe.
2. **G-1** (Next-terminator) — biggest crit/high FP class, pure structural check.
3. **G-11** (d20 comment/exit position) — clear correctness bug, ~6 obvious FPs, cheap.
4. **G-6** (virtual tables) — cheap allowlist.
5. **G-12** (d3 PK/FlowField/existence) — several clean sub-classes, modest effort.
6. **G-2** (runtime-implied tempness) — high volume, two structural sub-features.
7. **G-10 / G-3** (load-wrappers / interprocedural filter) — share a one-hop callee summary.
8. **G-5** (wrong table name) — correctness/trust; audit the binding.
9. **G-4 / G-7 / G-8** — lower volume / need investigation first.

All of these follow the epoch's suppression-direction discipline: only add `Known(true)` / skip
from an exact structural signal (terminator position, trigger-Rec, virtual-table allowlist,
IsTemporary guard, PK/FlowField field class), everything else stays firing.

---

## FP-rate summary (this CDO run, post-temp-state binary)

| Severity band reviewed | Findings | Confirmed real | False positive | FP rate |
|---|---|---|---|---|
| critical + high (primary) | 441 | 348 | 93 | ~21% |
| medium + low (primary) | 524 | 329 | ~195 | ~37% |

The medium/low band's higher FP rate is concentrated in G-9 (trigger-`Rec`) and the load-state
detectors — addressing G-9 alone would remove the largest single chunk.


---

# Iteration-2 gaps (G-13..G-19) — residual FPs found re-triaging the post-G1..G12 binary

Re-triaged all 768 post-fix primary findings against source (19 Sonnet batches) → ~110 residual
false positives (~14%, down from ~29%). Several are INCOMPLETE prior fixes; a few are genuinely
missed detectors. Ordered by volume x ease.

## G-13 — d10 (self-modifying-loop) and d39 never temp-gated (HIGH volume, easy)
**Symptom:** `d10-self-modifying-loop` fires on `Delete`/`Modify` of the iterating record even when
it is a `temporary` record — an in-memory cursor self-modify is safe (cursor corruption only
applies to physical SQL cursors). The temp-state epoch gated d1/d3/d33/d36/d37/d40 on `Known(true)`
but d10 (and `d39-record-left-dirty-across-chain`) were never added to that set.
**Evidence:** batches 1 (x3), 4, 13 (x2), 14, 17 — ~10 FPs (TempDOFileEDoc, temp worksheet line,
OpenDocFiles, DOFileExportedXml...). d39 on temp: batch 19 (DOFile).
**Approach:** add the same `Known(true)` temp gate the other detectors use to d10 and d39
(skip/suppress when the op's record is `Known(true)` temporary). Reuse the existing `op.temp_state`
check + (for d10) the path-resolved verdict. Suppression-direction safe.
**Status: FIXED (commit `fix(engine-g13): temp-gate d10 self-modifying-loop + d39 record-left-dirty (G-13)`).**
d10 skips ops whose `op.temp_state` is Known(true) (same gate as d33 — d10's findings are
direct in-routine cursor ops, so the raw op state is the right input; no cross-routine path
to resolve). d39 skips bindings whose `binding.source_temp_state` is Known(true) (same gate
as d40 — the subject is the CALLER's forwarded source record). Both gates are exact-match on
Known(true); physical and Unknown keep firing (controls in `tests/gap_g13_temp_gate.rs`
prove it). New `tempRecord` skip counter in both detectors' stats.

## G-14 — G-9 trigger set missed OnLookup / OnAssistEdit field triggers (HIGH volume)
**Symptom:** d11/d21/d37 still fire on `Rec` inside `OnLookup` and `OnAssistEdit` field triggers
(and field-level `OnValidate` lookups). G-9 covered OnValidate/OnAction/OnAfterGetRecord/
OnDrillDown/OnAfterGetCurrRecord but NOT OnLookup/OnAssistEdit — the AL platform loads `Rec`
before those too, and a `Validate` in OnLookup is persisted by the page framework.
**Evidence:** batches 12 (x4), 14 (x8), 15 (x4), 18 (x2) — ~18 FPs. residual-of-G9.
**Approach:** extend `is_platform_loaded_trigger_rec` (detectors/mod.rs) — add `OnLookup`,
`OnAssistEdit` to the page trigger-name set (and confirm field-level OnValidate is covered).
**Status: FIXED (commit `fix(engine-g14): extend trigger-Rec suppression to OnLookup/OnAssistEdit (G-14)`).**
`PAGE_TRIGGERS_REC_LOADED` now includes `OnLookup` and `OnAssistEdit` (field-level `OnValidate`
was already in the G-9 set). Proven by `tests/gap_g14_onlookup_triggers.rs` (suppression +
non-trigger and non-Rec controls); no golden moved.

## G-15 — d3/d42 fire on field WRITES, post-Init writes, and PK fields (medium)
**Symptom:** (a) `d3-missing-setloadfields` fires when the fields after a `Get`/`FindLast` are
WRITTEN (Init + assign + Insert / "if not Get then begin Init; ... end"), not read — writes need no
SetLoadFields. (b) An intervening `Init()`/`Clear()` resets the buffer, making the prior load
irrelevant. (c) `d42-cross-call-wrong-setloadfields` includes PRIMARY-KEY fields in the
must-be-loaded set (PK is always loaded — same as the G-12 fix, not applied to d42).
**Evidence:** batches 3 (write-only-after-get x1, d42 PK x3), 5 (get-false-writes), 6
(write-after-init), 12 (write-after-init) — ~8 FPs.
**Approach:** d3 — only count field READS as "accessed without load" (exclude write targets);
treat an `Init`/`Clear` between the Get and the access as resetting load relevance; also suppress
when the Get returns into a `if not Get then` construct-and-Insert. d42 — exclude PK fields
(reuse G-12's PK-exclusion helper).
**Status: FIXED (commit `fix(engine-g15): d3 ignores field-writes/post-Init; d42 excludes PK fields (G-15)`).**
d3 (a): the model records assignment LHS positions (`PVarAssignment` anchors at the statement
start == the LHS member expression) — a field access matching an assignment LHS by (position,
member name) is a WRITE target and no longer counts toward the witness; RHS reads keep firing.
This also covers the `if not Get then begin Init; ...; Insert end` construct shape without a
separate heuristic. d3 (b): `Init` record ops and `Clear(<var>)` bare calls close the
post-retrieval access window (`WINDOW_CLOSING_OPS`; `deriveLoadStates` unchanged — `Init`
keeps the SetLoadFields selection). d42 (c): the callee parameter table's PK (first key)
fields are dropped from `requiredLoadedFieldsAtEntry` via the shared
`primary_key_field_names_lc` helper (G-12's d3 exclusion, factored into detectors/mod.rs);
new `pkOnly` skip counter. Controls in `tests/gap_g15_d3_d42_writes.rs` prove genuine non-PK
normal-field reads (same-routine, RHS-of-assignment, pre-Init, and cross-call) still fire.

## G-16 — record loaded via deeper wrappers / record-assignment not recognized (medium)
**Symptom:** d11/d21 fire "never loaded" when the record was loaded via (a) a multi-hop or
non-Get-named wrapper (`FindTemplate`->`FindTemplateWithReportID`->FindSet;
`GetApplicationAreaSetupRecFromCompany(var,...)` facade; `InsertIfNotExists` which Gets-or-Inserts),
or (b) a record `:=` assignment from a loaded var (`Cust := Rec`, `EmailLog2 := EmailLog`).
**Evidence:** batches 8, 16 (x4 + assign x2), 18 (facade x2 + assign x1), 19 (InsertIfNotExists)
— ~10 FPs. residual-of-G10 (one-hop was too shallow) + NEW record-assign-as-load.
**Approach:** (a) extend G-10's callee-load summary to follow >1 hop (bounded depth) and recognize
Get/Find anywhere in the wrapper's transitive body on the by-var arg; (b) treat `RecB := RecA` as
RecB loaded when RecA is loaded (a load event in the intraprocedural load scan).

## G-17 — d33 still misses some one-hop filters + page selection filter (medium)
**Symptom:** `d33-unfiltered-bulk-write` still fires when the filter is set by (a) an IN-SOURCE
one-hop helper that G-3 should have caught (`SetEMailTemplateLineFilter`, `SetMergeFieldFilter`),
(b) a helper defined on a DEPENDENCY table (`SetTemplateFilter` in a .app), or (c)
`CurrPage.SetSelectionFilter(Rec)` (page row selection).
**Evidence:** batches 9 (in-source helper not recognized — a G-3 BUG; dep-table helper), 16
(SetMergeFieldFilter), 17 (CurrPage.SetSelectionFilter x2) — ~5 FPs.
**Approach:** debug why G-3's one-hop filter summary misses these in-source helpers (binding
resolution? the helper sets SetRange on the by-var arg — should match); add `SetSelectionFilter`
as a filter-applying builtin; dep-table helpers need the ABI side (lower priority).

## G-18 — d1 transitive loop FALSE attribution (correctness, low volume)
**Symptom:** d1 reports an op as in-a-loop when, on the real call path, it is NOT inside any loop —
the engine attributes a loop from a SIBLING call path of a shared callee. (Distinct from G-4, which
was wording for genuinely-per-iteration ops; here the op truly runs once.)
**Evidence:** batch 7 (eDocumentsConfigExists IsEmpty x2 — reached via a non-looping chain, loop
mis-attributed from RunReport's separate path).
**Approach:** audit the transitive loop-attribution / path construction — a loop must be on the
ACTUAL path to the op, not merely in some sibling call of an ancestor. Needs care; correctness fix.

## G-19 — temp via by-var param with no keyword, all callers temp (contentious / likely WONTFIX or source-fix)
**Symptom:** d1/d3/d10/d33 fire inside a callee on a `var Record X` param that LACKS the `temporary`
keyword, even though every resolved caller in this app passes a `temporary` local. Per-path the op
is `ParameterDependent`; with no single caller frame (intra-callee finding) it stays Unknown/fires.
**Evidence:** batches 1, 11 (x5 TempAut*), 15 (x4 CDO Temp Blob), 16 (x2), 19 — ~12 FPs.
**Assessment:** OPEN-WORLD CORRECT to fire (the callee could be called with a physical record
elsewhere). Two non-suppression options: (1) whole-program closed-world — if ALL resolved callers
pass temp, treat the param as temp (precision, app-scoped); (2) recommend the SOURCE add the
`temporary` keyword to the param (contract-trust then makes it Known(true)). Likely defer or treat
as a precision feature, not a clear bug. Do NOT hard-suppress.

## Iteration-2 fix order
1. **G-13** (d10/d39 temp-gate) — high volume, trivial (reuse temp gate).
2. **G-14** (OnLookup/OnAssistEdit) — high volume, extend the G-9 set.
3. **G-15** (d3 writes/Init/PK + d42 PK) — clean sub-classes.
4. **G-16** (deeper load-wrappers + record-assign) — extend G-10.
5. **G-17** (d33 one-hop bug + SetSelectionFilter) — debug G-3 + add builtin.
6. **G-18** (transitive false-attribution) — correctness, careful.
7. **G-19** — investigate; likely defer/source-fix (do not hard-suppress).
