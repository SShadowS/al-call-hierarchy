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

## Suggested order

1. **G-1** (Next-terminator) — biggest FP class, pure structural check, low risk.
2. **G-6** (virtual tables) — cheap allowlist, clean class.
3. **G-2** (runtime-implied tempness) — high volume, two structural sub-features, suppression-safe.
4. **G-3** (interprocedural filter) — needs a one-hop summary; medium effort.
5. **G-5** (wrong table name) — correctness/trust; audit the binding.
6. **G-4 / G-7 / G-8** — lower volume / need investigation first.

All of G-1..G-3,G-6 follow the epoch's suppression-direction discipline: only add `Known(true)`
/ skip from an exact structural signal (terminator position, virtual-table allowlist, IsTemporary
guard), everything else stays firing.
