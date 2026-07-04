# PageExtension-merge + final-residual Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

> Status: **v2.1** (round 2: both GO-WITH-CHANGES on wording-drift only — the superseded T2 text SCRUBBED from the body,
> the aggregate/qualified/no-source fixtures + the T1 stop-gate propagated into the checklists. The addenda below are BINDING and
> supersede conflicting task text).

## Round-1 review addenda (BINDING)

**T2 — the narrowing is GLOBAL, not collision-scoped (both reviewers CRITICAL):**
- A compiler-proven instance-only name (no bare-call form anywhere in AL) is removed from the bare-builtin candidate
  set **unconditionally** — in every object kind and context, not just "where a table-scope candidate exists". A bare
  `Run()` with NO source candidate → Unknown (today's Builtin resolution there is itself a false-Catalog vector this
  task FIXES). Qualified forms (`CurrPage.Update()`, `PageVar.RunModal()`, `Report.Run(...)` statics) unaffected.
- **The compiler-grounding matrix** (before any catalog change; per name × context): page trigger/action/procedure,
  pageextension, table/tableextension, report/reportextension (+`CurrReport` analogs), XMLport, codeunit OnRun. A name
  proven no-bare in ALL contexts → globally suppressed; ANY context accepting a bare form → context-specific or left
  colliding. `Update` is the riskiest (CurrPage.Update idiom) — extra care. Negative fixture: bare `Run()` with no
  source candidate → Unknown (NOT Builtin).
- Phrase the implementation in RESOLVER terms (candidate-set filtering during bare-call resolution), not trigger terms.

**T1 — closure direction + collision reality (both CRITICAL):**
- The merge's visibility set: base page routines ∪ pageextension routines **where the EXTENSION's defining app is
  visible from the CALLER's dependency closure** AND the member access check passes (internal = same-app/friend per
  the existing internalsVisibleTo model; local = never; protected = the existing protected rules). Never
  receiver-object-closure-anchored; never whole-snapshot-unfiltered.
- **Aggregate-then-adjudicate:** collect ALL visible candidates (base + every visible extension) FIRST, then feed the
  existing overload/ambiguity machinery. No base-first or extension-order-first resolution. Base-vs-extension exact
  duplicate signatures are COMPILE ERRORS in AL (AL0115; cross-extension AL0226) — the ambiguity fallback is
  DEFENSIVE-ONLY against malformed source, state it in the code doc.
- **Immediate stop-gate:** if MemberNotFound is NONZERO after T1's harness run, STOP the plan and present the residual
  before T2 (do not let later tasks obscure the causal readout).

**T3 — strict prerequisites (both):**
- T3 lands strictly AFTER T1 (an inner call may resolve through the new merge — fixture: an inner
  pageextension-declared member types ONLY when T1 yields a single visible route; multiple extension candidates →
  decline) and AFTER T2 (the builtin return catalog is a PASSIVE dictionary trusting `resolve_bare`'s Builtin verdict —
  T2's global suppression is the prerequisite that makes that verdict trustworthy). Shadowed-name fixtures mandatory
  (`Format`/`CopyStr` shadowed by source procedures with incompatible returns → the catalog must NOT type them).
- The Primitive-decline bypass keeps every other guard verbatim (shadow, WithState, abi_overload_collapsed,
  return_type_id, Variant/var, no recursion into pick_candidate).

**Cross-task:**
- Metrics are EXPECTATIONS, not promises: "expect MemberNotFound 7→0", "target floor 0 primary" — ratchets re-derived
  only from MEASURED values + the adjudicated ledger; any grounding failure → the honest residual stated and pinned.
- Report/ReportExtension: decide explicitly after index inspection (merge now or dated deferral with the reason);
  QueryExtension has a different surface — no parity implied without evidence.
- Boolean-op fixtures cover each lowered token class (Eq/Ne/Lt/Le/Gt/Ge/And/Or/Xor/In) + an arithmetic and a
  text-concat decline.
- Narrative corrections are APPEND-ONLY errata (dated correcting entries in CHANGELOG/charter/memory; no silent
  rewrites of plan-10 claims). Eleventh resolution arc (master `9b5f3de`, CDO primary real-`unknown`
> 0.0497% / 9: MemberNotFound 7, UntrackedReceiver 1, BuiltinPrecedenceCollision 1; `ambiguousResolved=7`;
> `genuine_wrong=0`). Two grounding reports (this session) — the first FALSIFIED the plan's original premise, the SECOND
> time this class of lesson has struck (plan 9's "13 workspace absences" were a catalog gap; now):
> **The 7 "verified-real eCandidates absences" are NOT absences.** CDO's own workspace ships
> `Al/Extensions/eCandidates/CDOConnecteCandidates.PageExt.al` (`pageextension 6175296 ... extends "CTS-CDN Connect
> eCandidates"`) declaring ALL THREE missing members (`GetOutputProfile` :74, `OnlyCustomersAreHandled` :299,
> `OnlyVendorsAreHandled` :310 — `internal`, same app as the caller = visible). The plan-8-era "verified absent" check
> inspected only the BASE page inside the dependency `.app`. THE ENGINE GAP: `resolve_member`'s `ReceiverType::Object`
> arm (`resolver.rs:1986-2091`) calls `resolve_in_object` directly — NO base∪extension merge. The Table analog exists
> (`resolve_in_table_scope` `resolver.rs:954`, `table_extensions_of` `index.rs:515`); a PageExtension's routines are
> indexed under the extension's own id (`node_extract.rs:458-462`) and are structurally unreachable from a base-typed
> receiver. Fix the merge → expect most/all 7 → Resolved/Source. **ProvenAbsent machinery is DEFERRED contingent on
> T1's re-measure** — if the population empties (expected), building it now would be taxonomy-without-population
> (YAGNI + the twice-learned lesson: MEASURE the population before building taxonomy for it).

**Goal:** Close the PageExtension-merge engine gap (the 7), the Page implicit-Rec field arm (the 1), assess the
collision narrowing (the 1), and land call-result/boolean arg typing (ambiguousResolved 7 → ~4) — driving real-unknown
toward the floor, `genuine_wrong=0`, zero false edges, and correcting the "verified-real absences" narrative honestly
in CHANGELOG/charter/memory.

**Tech Stack:** Rust. No `engine::l3`/`l2` imports in `src/program/resolve` (grep-guarded). FOREGROUND cargo. Full CDO
harness per task (`CDO_WS="U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud"` + `ENFORCE_CDO_WS=1`,
`--test-threads=1`). Clippy `--all-targets` clean. The per-task L3 preflight site ledger discipline (plan-10 precedent)
applies to every CDO movement.

## Key facts (verified on `9b5f3de`; the grounding reports are authoritative)

- **T1 (the merge):** mirror `resolve_in_table_scope` for Page/PageExtension in the `Object` arm BEFORE the
  instance-builtin catalog fallback: base Page's routines ∪ every closure-visible PageExtension's routines (the
  `table_extensions_of` pattern — whole-snapshot, dependency-closure-filtered; an out-of-closure extension is
  AL-invisible). Visibility filtering per the existing access machinery (internal + same-app = visible — the 7's
  shape). Overloads across base+extensions feed the SAME ambiguity/pick machinery (a base-vs-extension same-name
  same-arity pair = genuine ambiguity → AmbiguousResolved, not first-wins). ALSO: check the REPORT/QUERY analogs
  (reportextension exists in AL — does the index have report_extensions? If the merge pattern generalizes cheaply, do
  Report too; if not, dated note). The 7 sites' fix expectation: all → Resolved (internal, same-app). fresh/L3: L3
  likely ALSO missed these (verify per the ledger — matches vs fresh_extra).
- **T2 (the Page field + the collision):** (a) widen Step 3a's implicit-self gate (`receiver.rs:892-919`, currently
  Table|TableExtension) to Page|PageExtension via the EXISTING `implicit_rec_table_id` (`resolver.rs:1072-1087` — has
  the Page→source_table + PageExtension arms; used by resolve_bare Step 3 today); same with_state +
  `table_scope_has_routine` guards; the known site: `"View (Blob)".CreateInStream(...)` on Page 6175411
  (SourceTable field(28) Blob — verified). (b) the BuiltinPrecedenceCollision: bare `Run()` in a page action trigger
  (`CDOEMailJobs.Page.al:125`) vs the SourceTable's own `procedure Run()` (`CDOEMailJob.Table.al:192`) — the
  PROBE-THEN-DECIDE guard fires because `run` is in PAGE_INSTANCE and in the 785-name GLOBAL_BUILTIN_METHODS union;
  but `Run`/`RunModal`-class names are INSTANCE methods (Codeunit/Page/Report), never bare-callable globals (the
  catalog's flat-union inheritance, not per-name verification — its own doc admits it). THE FIX IS GLOBAL (per the
  binding addenda — the collision-scoped variant is SUPERSEDED): a compiler-grounded (per-name × per-context matrix)
  instance-only name is removed from the bare-builtin candidate set UNCONDITIONALLY, in every object kind — so the
  table's own procedure wins where one exists, and a bare `Run()` with NO source candidate → Unknown (fixing today's
  false-Builtin vector there too). Qualified forms (`CurrPage.Update()`, `PageVar.RunModal()`, `Report.Run(...)`)
  unaffected. If the grounding is uncertain for ANY name in ANY context, leave that name colliding/probing (honest).
- **T3 (arg-typing increments — the second grounding's scope):** (a) member-call-result args: a new
  `ExprKind::Call` arm in `type_one_arg` (`arg_dispatch.rs:344-492` — Call{function:Member{...}} and bare-Identifier
  function both); reuse Step-5/6's guards verbatim (shadow guard, `resolve_bare`/`resolve_member` SINGLE-route
  contract = the inner-uniqueness rule; no recursion into pick_candidate) but BYPASS only the Primitive-decline
  (`receiver_from_routine_node`'s `receiver.rs:1838-1840` — args WANT primitives; keep the
  `abi_overload_collapsed`/`return_type_id` guards verbatim); WithState-gated; `var_passable:false`. Verified-real
  yield: the 2 `PrintPDFFile` sites (`Page 6175389:239/:252` — `DOTempBlob.ToBase64String()` → workspace
  `Table 6175296` single-overload arity-0 returns Text → exact-eliminates the 2 Record-typed siblings). (b)
  Binary/Parenthesized bool typing: comparison/logical ops (`Eq,Ne,Lt,Le,Gt,Ge,And,Or,Xor,In`) are UNCONDITIONALLY
  Boolean in AL — type without operand inspection; arithmetic + `Other` decline; `Parenthesized` unwraps recursively.
  (c) a minimal builtin return-type catalog (`strsubstno→text`, `format→text`, `copystr→text`, `lowercase/uppercase→
  text`, `round→decimal`, `strlen→integer` — framework_returns-style, per-entry cited, fail-closed absences), gated on
  `resolve_bare` positively reporting `RouteTarget::Builtin` (never name-string matching alone). (d) **the
  remaining-ambiguous dump diagnostic** (`#[ignore]`d, the flip-dump precedent) so the residual is mechanically
  re-groundable. HONEST YIELD: expect ambiguousResolved 7→~4-5 — sites #1/#7/#8's OUTER receivers (`CTS-SYS
  Telemetry`, `AOAI Chat Messages`) are SymbolOnly (not embedded) and the tier gate (`resolver.rs:543`) never attempts
  the pick regardless of arg typing — they stay honest; ABI param retention is the separately-tracked lever.
- **T4 (contingent close):** re-measure after T1-T3. IF MemberNotFound emptied → ProvenAbsent DEFERRED with the full
  design recorded (the grounding's §1a/§1b: the `Route::proven_absent` marker + `ObligationOutcome::ProvenAbsent` +
  the 8-obligation proof table + the recoveredFiles-consult invariant + the app_content_hash anchoring) as the
  blueprint for a future population; the plan-10 "verified-real absences" narrative CORRECTED in CHANGELOG + charter +
  the session memory (the twice-learned lesson recorded). IF a residual survives T1 → STOP and present the residual to
  the user before building ProvenAbsent (scope decision).
- The union-read pin exists (`lower/mod.rs:1668-1706`, plan-9 T3); embedded `.app` source goes through the SAME
  al_syntax lowerer (`embedded.rs:54-74` → `parse.rs:104`) — same union-read + ParseStatus semantics; the CTS-CDN and
  PageExt files are both Clean (not among the 8 recoveredFiles).

## Global Constraints

- `rustfmt <file>` per-file — NEVER `cargo fmt`. Stage only named files — NEVER `git add -A`. CHANGELOG per task.
  Gates per task: clippy `--release --all-features --all-targets -- -D warnings`, `cargo fmt --check`,
  `cargo test --workspace`, the FULL CDO harness, the L3 preflight site ledger (blocks on wrong/unexplained).
- **Soundness cardinal:** the merge is closure- and visibility-filtered (never resolve to an invisible extension
  member); base-vs-extension overload collisions feed the genuine-ambiguity machinery; the collision narrowing is
  compiler-grounded per-name or left colliding; all new arg-typing feeds the UNMODIFIED pick_candidate guard stack;
  `genuine_wrong=0` hard.
- **Correctness over compatibility:** ratchets re-derived DOWN dated; the false "verified-real absences" narrative
  corrected wherever it lives (CHANGELOG plan-10 entry, charter §, the memory) — the historical entries get a
  correcting note, not silent rewrites.
- Out of scope: ProvenAbsent MACHINERY (contingent-deferred per T4); ABI param retention; the 2 grammar defects; the
  .dependencies double-include; implicit conversions; Report/Query extension merges IF not cheap (dated note).

## Tasks

### Task 1: The Page/PageExtension routine merge (the 7)
**Files:** `resolver.rs` (the merge in the Object arm), `index.rs` (a `page_extensions_of` analog if absent), fixtures.
- [ ] Failing fixtures: a base-Page-typed receiver calling a PageExtension-declared internal procedure (same app) →
  Resolved/Source; a DIFFERENT-app internal extension member → declines (visibility); an out-of-closure extension →
  invisible; TWO caller-visible pageextensions both declaring the viable member → ambiguity, no first-wins (the
  aggregate-then-adjudicate proof); a base-vs-extension same-name-same-arity pair → the ambiguity machinery
  (defensive-only — AL0115 makes it uncompilable, state in the fixture doc); base-only unchanged; the arity-mismatch →
  ArityMismatch (name found).
- [ ] Implement (mirror resolve_in_table_scope; CALLER-closure-anchored per the addenda; check Report/ReportExtension
  generalizes cheaply — do or date-note).
- [ ] FULL CDO harness + the ledger (all 7 sites): expect MemberNotFound 7→0 (all internal-same-app → Resolved),
  unknown 9→2 (0.011%); adjudicate vs L3 per the discipline; ratchets dated; `genuine_wrong=0`. **STOP-GATE: if
  MemberNotFound is NONZERO after this harness run, STOP the plan and present the residual before T2.** Commit:
  `fix(resolve): merge PageExtension routines into base-Page member resolution — the missing scope analog (Task 1)`.

### Task 2: The Page implicit-Rec field arm + the collision narrowing (the 2)
**Files:** `receiver.rs` (Step 3a widening), `resolver.rs`/`member_catalog.rs` (the narrowed probe), fixtures.
- [ ] Failing fixtures: (a) `"View (Blob)".CreateInStream(X)` in a Page-with-SourceTable procedure → resolves via
  implicit-Rec field chain; PageExtension variant; with-unproven + routine-shadow declines; a Page WITHOUT SourceTable
  → declines. (b) THE GLOBAL SUPPRESSION (per the addenda): bare `Run()` where the SourceTable declares `Run()` →
  the table's procedure; bare `Run()` with NO source candidate → Unknown (NOT Builtin — the negative); the same in a
  CODEUNIT context (global, not page-scoped); qualified forms preserved (`CurrPage.Update()`, `PageVar.RunModal()`,
  `Report.Run(...)` statics still resolve); a genuinely-global name (`Message`) still probes as builtin; an
  uncited/uncertain name stays colliding (honest); the per-name×context grounding matrix documented in the report.
- [ ] Implement (resolver-terms candidate-set filtering); FULL CDO harness + ledger: EXPECT (not promise)
  UntrackedReceiver 1→0, BuiltinPrecedenceCollision 1→0 (target floor 0 primary — ratchets only from MEASURED values;
  any grounding failure → the honest residual stated and pinned); `genuine_wrong=0`. Commit:
  `fix(resolve): Page implicit-Rec fields + bare-call builtin probe narrowed per compiler grounding (Task 2)`.

### Task 3: Call-result + boolean arg typing (ambiguousResolved 7→~4)
**Files:** `arg_dispatch.rs` (the Call/Binary/Parenthesized arms + the builtin catalog), the dump diagnostic, fixtures.
- [ ] Failing fixtures: same-object bare-call-result arg (`Foo(GetCount())` — wire the orphaned `-callexpr-discriminator`
  bank); member-call-result (`Foo(X.Method())` — the PrintPDFFile shape); the inner-overload-set decline (2 same-arity
  inners → untyped); a SymbolOnly-inner decline; Binary comparison → Boolean; arithmetic declines; Parenthesized
  unwraps; the builtin catalog (`StrSubstNo(...)` → text; an uncataloged builtin → untyped); Variant/var gates
  unchanged (the stack applies).
- [ ] Implement + the remaining-ambiguous dump diagnostic; FULL CDO harness + ledger: record exactly which of the 7
  flip (expect the 2 PrintPDFFile; adjudicate each pick compiler-correct; the SymbolOnly-blocked sites documented
  honestly); ambiguousResolved pins re-derived; `genuine_wrong=0`. Commit:
  `feat(resolve): call-result and boolean argument typing — member-call results, builtin returns, bool exprs (Task 3)`.

### Task 4: Measure + close (+ the contingent ProvenAbsent decision)
- [ ] Full re-measure; IF MemberNotFound==0: record the ProvenAbsent blueprint as DEFERRED-with-design (the grounding's
  proof-obligation table verbatim in the plan/report; the recoveredFiles invariant stays pinned); IF residual: STOP,
  present to the user. The narrative correction: CHANGELOG (a correcting entry re plan-10's "verified-real absences"),
  charter memory + MEMORY.md (the twice-learned falsified-premise lesson: measure populations before taxonomy),
  session memory update. Ratchets at floors, dated. CHANGELOG capstone; DEFERRED roadmap visible (ABI param retention
  now the ambiguousResolved lever; the 2 grammar defects; the double-include; Report/Query merges if noted; implicit
  conversions; protected Variables[]; Sender param-TYPE; Step-4b WithState symmetry). Commit:
  `docs(resolve): pageext-merge arc complete — real-unknown 0.05%→~0.01% (Task 4)`.

## Roadmap — beyond this plan
ABI param-type retention (SymbolOnly dispatch — the remaining ambiguousResolved lever); ProvenAbsent (blueprint
recorded, awaiting a real population); the 2 tree-sitter-al grammar fixes; the .dependencies double-include root cause;
implicit conversions; protected Variables[]; Sender param-TYPE validation; Step-4b WithState symmetry (opus A).
