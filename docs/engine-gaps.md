# Engine detector-quality gaps (backlog)

False-positive / quality classes surfaced by triaging the **post-temp-state-epoch** binary
against a real shipping app (Continia DocumentOutput / Cloud, ~2260 findings; 441 crit+high
in primary scope reviewed against source → 93 false positives, ~21%). The temp-state epoch
fixed the *declared*-temp class (the `ClearFiles` d33 critical is gone); these are the
classes that remain. Ordered by volume × ease.

Each gap: symptom, evidence (which triage batches + example sites), suggested approach.
This is a backlog for later work, not a committed plan.

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

**Symptom:** `d1` fires on reads of BC system virtual tables (`AllObjWithCaption`, `Field`)
which have no physical SQL backing (they read BC's internal metadata store). Engine marks them
"type not loaded" and fires conservatively.

**Evidence:** batch 5 (AllObjWithCaption 223/233/239, Field 666/667/677). 6 FPs.

**Approach:** maintain a small allowlist of known virtual/system tables (AllObjWithCaption,
Field, Object, Integer, Date, etc.) classified non-physical → SQL-cost detectors skip them.
Cheap, eliminates a clean FP class.

## G-7 — dead code via commented-out call site not detected

**Symptom:** `d1`/`d14` fire on a routine whose only caller is commented out — the routine is
dead, the finding moot.

**Evidence:** batch 4 (CDO Data Upgrade `UpgradeOutputProfileOnDocsWorker`:115; its caller
`UpgradeOutputProfileOnDocs` is commented out at line 17).

**Approach:** the parser already drops commented code, so the call edge is correctly absent —
the gap is that reachability/dead-routine analysis didn't down-rank the finding. Cross-detector:
when d14 marks a routine dead, other detectors' findings rooted only in that routine could be
suppressed or down-confidenced. Low volume; lowest priority.

## G-8 — residual global / by-var-param temp resolution gaps

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
