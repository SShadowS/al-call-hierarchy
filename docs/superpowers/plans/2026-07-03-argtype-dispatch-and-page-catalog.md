# Arg-type dispatch + Page/Report instance-catalog completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

> Status: **v2.1** (round 2: gemini GO, gpt GO-WITH-CHANGES → closers folded below. BOTH addenda sections are BINDING
> and supersede conflicting task text).

## Round-2 closers (BINDING)

- **C5 — `var` params use ByRef-EXACT identity:** the length-stripping rule applies ONLY to by-value dispatch
  compatibility; for a `var` param the arg's declared type must match EXACTLY (length included; `var Text[30]` arg vs
  `var Text[50]` param → NO PICK). Negative fixture mandatory.
- **C6 — literal typing is CANDIDATE-SET-AWARE:** a literal arg carries `LiteralKind`, and the picker DEGRADES whenever
  the candidate set contains any target family the literal is contextually usable as but unproven: a STRING literal with
  any `Code`/`Char` candidate at that position → no pick; an INTEGER literal with any `Decimal`/`BigInteger` candidate →
  no pick — unless that exact pair is compiler-fixture-proven (the `ws-overload-collision` Integer-vs-Code[20] pair IS
  the proven exemplar: an Integer literal cannot bind Code[20], so eliminating Code is sound — document per-pair).
  Negatives: string-literal Text-vs-Code, numeric Integer-vs-Decimal.
- **I7 — the T2 plumbing carries `ArgDispatchInfo`** (canonical semantic type, LiteralKind/origin, var-passable flag,
  degradation state) per position — NOT bare `Option<String>`; the param side carries canonical type + mode. The task
  body's `Vec<Option<String>>`/`normalize_type_text ==` wording is superseded.
- **I8 — the T1 fixture list is corrected:** `PageVar.SaveRecord` is a NEGATIVE/control (stays Unknown — CurrPage-only);
  the positive SaveRecord fixture sits on the CurrPage receiver path, and (gemini) the receiver's CURRPAGE-ORIGIN must be
  an explicit flag — never inferred from the resolved page type (a PageVar of the same page id must not leak SaveRecord).
- **I9 — "discriminating position" is defined from the FULL candidate set BEFORE any compatibility filtering** (a
  Variant-bearing candidate degrades the call even if it would be eliminated first).

## Round-1 review addenda (BINDING)

**T2 — the pick rule, fully hardened (gpt C1-C3, I3-I5; gemini 3-5):**
- **Call-level degradation:** a pick requires ALL supplied args typed (`Some`) AND every candidate's full param
  type+MODE metadata known. ANY untyped arg, missing candidate metadata, SymbolOnly candidate in the set, degraded
  candidate, or arity uncertainty → NO PICK (stays `AmbiguousResolved`). Never filter unknown-metadata candidates out of
  the competition. ("Untyped-never-eliminates" is superseded by: an untyped position degrades the whole call.)
- **Dispatch identity ≠ text identity:** compare on a DISPATCH-canonical form, not raw sig_fp text: (a) Text/Code length
  brackets are NON-DISCRIMINATING — strip them for compatibility, and if stripping makes ≥2 candidates identical at any
  discriminating position → DEGRADE (no pick; `(Text[30])` vs `(Text)` can never be picked-between); (b) object-bearing
  types (`Record`/`Page`/`Report`/`Codeunit`/`Enum`/`Interface` + subtype) canonicalize via the EXISTING fail-closed
  object resolution (`resolve_object_ref` — semantic identity, so `Record "Sales Header"` == `Record 36` when they
  resolve to the same table; unresolvable/ambiguous → that position is untyped → degrade); (c) scalar families compare
  by exact base keyword only (integer≠decimal≠biginteger — no implicit-conversion modeling).
- **Parameter MODE:** carry `var`-ness. A literal/call-result arg is NOT var-passable → a `var` param is incompatible
  with it (a sound elimination); a declared var arg is compatible with both modes. If mode metadata is missing → degrade.
- **`Variant` wildcard:** any candidate with a `Variant`(/`Any`) param at a discriminating position → DEGRADE (no pick)
  until compiler-fixture-proven precedence exists. Fixture: `(Variant)` vs `(Text)` with a Text arg → no pick.
- **Literal typing per fixture-proven family only:** Integer literal → `integer`; string literal → `text`; boolean;
  decimal-with-point → `decimal`. Each family's flip fixtures state the compiler argument (e.g. `5` cannot bind
  `Code[20]` → the Integer overload is the true answer). Unproven literal shapes → `None`.
- **Caller-scope lookup:** the arg var-typing helper uses EXACTLY the caller's scope chain (params → locals → globals,
  the same shadowing as Step 2's receiver lookup, never a receiver/with-block scope). Shadowing fixtures (local-over-
  global, param-over-global) mandatory.
- **M1 reword:** "stricter-than-AL" is safe ONLY when it yields no-pick; any rule that singles out one candidate among
  compiler-compatible alternatives is unsound — hence the degrade-on-indistinguishable rules above.
- Negative fixtures (mandatory): Text[N]-vs-Text, Code[N]-vs-Code, Record-name-vs-id, var-param-with-literal,
  Variant-vs-exact, shadowing, mixed known/unknown-metadata candidate set.

**T1 — catalog segmentation + proof (gemini C2, gpt I1-I2):**
- **`SaveRecord` is CurrPage-ONLY** (a compiler error on a page VAR) — it is NOT added to the general Page instance
  catalog; only the CurrPage receiver path gets it (verify how CurrPage receivers are typed; no CDO site uses SaveRecord
  — the 18 are SetTableView/SetRecord/GetRecord only). General page vars get SetTableView/SetRecord/GetRecord/
  SetSelectionFilter; Report vars get SetTableView.
- Per-method version/support matrix (MS Learn citation + the L3-catalog precedent per entry; runtime-gate or document
  the minimum BC target).
- The golden adjudication table compares ROUTE IDENTITY per site (call text, receiver type, expected target member, L3
  target, fresh target) — bucket movement alone insufficient.
- The 7 eCandidates sites: the ratchet notes must NOT imply "proven absent" — they remain Unknown diagnostics.

**T3 — the ParseStatus gate, scoped honestly (gemini C1 rebutted-in-part, gpt I6):**
- Blanket "any `#if` file → Unknown" is REJECTED (it would wholesale-degrade normal files; `#if` union-read is
  pre-existing, engine-wide, charter-documented semantics — resolution-confidence over dead branches is a KNOWN,
  documented over-approximation, out of this plan's scope to re-litigate).
- The REAL gate: `ParseStatus::Recovered` (actual parse-recovery data loss). This plan ships: the per-file Recovered
  diagnostic (count + FILE PATHS, surfaced additively), a documented invariant that any future absence/proven-class
  claim MUST consult it, and per-site flip adjudications note Recovered status. The full per-file resolution gate is
  deferred with a dated note (no absence-claims exist yet to gate).
- **Conflicting conditional singular properties (gpt C4):** `#if A SourceTable=X #else SourceTable=Y #endif` — after
  the flat-loop fix captures both, the PROGRAM layer must DEGRADE on conflict: implicit-Rec table becomes ambiguous →
  `Record{table: None}` (fail-closed), never first/last-wins; same for conflicting `implements` (no confident interface
  dispatch on unioned conflicting clauses). Program-level fixtures, not just lowering tests.
- T2's baseline is POST-T1 (remeasure between tasks; per-task CDO runs as always). Ninth resolution arc (master `25928f6`, CDO primary real-`unknown` 0.52% /
> `unknown=95`: CompoundReceiver 51, MemberNotFound 25, UntrackedReceiver 18, BuiltinPrecedenceCollision 1;
> `ambiguousResolved=56` carried candidate sets; `genuine_wrong=0`; legacy-comparable 0.83%). Two grounding reports (this
> session) fixed the scope — INCLUDING FALSIFYING the original "ProvenAbsent for the 13" idea:
> 1. **The 13 workspace-tier `MemberNotFound` are NOT absences** — all 13 (and 5 embedded siblings) are ONE deliberate
>    engine catalog gap: `is_metadata_sensitive_instance_method` (`resolver.rs:1651-1660`) excludes
>    `Page.{SetTableView,SetRecord,GetRecord,SetSelectionFilter,SaveRecord}` + `Report.SetTableView` — REAL, always-present
>    platform intrinsics (L3's own catalogs have them; the targets verified to exist). Building `ProvenAbsent` for them
>    would codify a FALSE claim. THE FIX: **catalog completion** — 18/25 sites become genuinely `Resolved`/Catalog
>    (95→~77), an ordinary correctness fix (no metric-definition ceremony). The remaining 7 (`CDOeCandidatesEventHandler`
>    → CTS-CDN page members that PROVABLY don't exist in the installed dependency — a real API-contract break in the
>    workspace's own code) stay honest `Unknown(MemberNotFound)` diagnostic-only — the documented future `ProvenAbsent`
>    prototype site.
> 2. **Arg-type dispatch** (the promoted lever): pick among the 56 `ambiguousResolved` candidate sets by typing the call
>    arguments — FAIL-CLOSED (exact-normalized-match only; pick iff exactly one compatible; anything else stays
>    `AmbiguousResolved`, which is already honest). Source-tier-only increment (ABI param types not retained — tier-gate
>    SymbolOnly out).
> 3. **The preprocessor fixtures** (the recorded prerequisite): union-read verified TRUE for objects/routines/globals —
>    pin it; two newly-found NON-union-read gaps (a `#if`-wrapped `implements` clause is silently skipped
>    (`lower/mod.rs:200-222` flat loop); a `#if`-wrapped object PROPERTY (e.g. SourceTable) is dropped (`:168-173` flat
>    loop)) — fix or explicitly pin as documented gaps; the `ParseStatus::Clean` precondition is UNENFORCED (zero
>    consultation under `src/program`) — decide + wire the gate.

**Goal:** Close the Page/Report instance-catalog gap (18 sites → Resolved), land fail-closed arg-type dispatch (some of
the 56 → single Resolved routes — each flip fixture-proven compiler-correct), and pin the preprocessor soundness
foundations — with `genuine_wrong=0` and zero false `Source`/`Catalog`.

**Tech Stack:** Rust (edition 2024). No new dependency. No `engine::l3`/`engine::l2` import in `src/program/resolve`
(grep-guarded; L3 catalogs/type-rel are PRECEDENT to port, never import). FOREGROUND cargo ALWAYS. Full CDO harness
(`CDO_WS="U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud"` + `ENFORCE_CDO_WS=1`, `--test-threads=1`) per task — never
a curated subset.

## Key facts (verified on `25928f6` — the two grounding reports are authoritative; anchors below)

**T1 — catalog completion:**
- `is_metadata_sensitive_instance_method` (`resolver.rs:1651-1660`): Page excludes settableview/setrecord/getrecord/
  setselectionfilter/saverecord; Report excludes settableview. The original rationale (can't validate the ARGUMENT)
  conflates argument-type soundness with member EXISTENCE — the resolver validates no catalog method's args today.
- All 13 workspace + 5 embedded `MemberNotFound` sites enumerated + verified: the targets exist (e.g.
  `Page 6175316 "CDO Fields (With Relation)"`; L3 `PAGE_INSTANCE` `member_builtins.rs:735-751`).
- **Golden interaction (CRITICAL, opposite shape from plan 8):** L3 RESOLVES these 18 — fresh is currently BEHIND. The
  fix moves them toward `matches` (not fresh_extra). Verify per-site against the frozen golden BEFORE landing; any
  target mismatch = investigate (the audit gate's `genuine_wrong=0` is the tripwire); `fresh_missing` may DROP.
- The 7 eCandidates sites: `OnlyVendorsAreHandled`/`OnlyCustomersAreHandled`/`GetOutputProfile` verified ABSENT from the
  installed `CTS-CDN Connect eCandidates` page source at ANY visibility (`.alpackages/...29.0.0.101335.app`). Stay
  `Unknown(MemberNotFound, embedded_source)`; document as the real ProvenAbsent prototype population (deferred).

**T2 — arg-type dispatch:**
- Plumbing gap (the receiver_text-pre-T2 pattern again): `collect_calls_v2` (`extract.rs:508-536`) has `arg_ids:
  Vec<ExprId>` in scope but stores only `arity`. Fix: `args: Vec<ExprId>` on `RawSiteV2` (`extract.rs:130-150`);
  PRECOMPUTE `arg_types: Vec<Option<String>>` ONCE in `resolve_call_site_obligation` (`full.rs:200-218` — it already owns
  `routine`/`obj.globals`/`file`) and thread just the strings down through `resolve_bare`/`resolve_member` →
  `resolve_in_object`.
- Arg typing (first increment, cheap+confident): literals (`al_syntax::ir::Literal` `expr.rs:62-73` → canonical type
  text — ~10-line match) + bare var/param/global (extract the Step-2 lookup as a helper, `receiver.rs:649-666`).
  DEFERRED: call-result/`Rec.Field`/`Enum::Value` args (a later increment; untyped positions never eliminate).
- Candidate param types: SOURCE tier already accessible — `BodyMap::get(rid) -> &RoutineDecl` → verbatim `params[].ty`
  (`body_map.rs:71-73`; `resolve_in_object` already has `body_map`). ABI: NOT retained post-ingestion → tier-gate the
  whole feature `obj_tier != SymbolOnly` (clean skip, not partial).
- The compatibility rule: for each typed position, `normalize_type_text(arg) == normalize_type_text(param)` (sig_fp's
  normalization — make it `pub(crate)`, one line; keeps `Text[10]≠Text[200]` — stricter-than-AL is SAFE, under-matches
  never over-match). Candidate compatible iff EVERY typed position matches; PICK iff exactly one compatible; 0/>1/any
  degraded → today's `AmbiguousResolved` unchanged. NO implicit-conversion modeling (Code↔Text etc. all decline —
  documented).
- Insertion: `resolve_in_object`'s `_` arm AFTER the prevalidation (`resolver.rs:504-532`), BEFORE candidate-route
  construction; a pick emits the normal single `(Exact, vec![make_routine_route(picked,…)])` — identical to the
  lone-candidate path, no new taxonomy.
- Test bank: **seven pre-authored ORPHANED fixtures** (`tests/r0-corpus/ws-overload-arg-type`, `-arg-pos2`, `-char`,
  `-enum-discriminator`, `-field-discriminator`, `-callexpr-discriminator`, `-negatives` — commit `b4ff081`, zero
  references) — wire the ones increment-1 covers (arg-type, arg-pos2, negatives); the enum/field/callexpr ones become
  the deferred-increment negatives (assert they DON'T flip yet). The pinned `ws_overload_collision_ambiguous_call_...`
  test WILL flip to Resolved (`Resolve(5)`: Integer literal exact-matches only `Resolve(X: Integer)` — the true compiler
  answer; rebaseline it as a strengthening).
- Blast radius: the `ambiguousResolved == 56` exact pins drop to the new measured value; each CDO flip's fresh/L3 bucket
  movement adjudicated PER-SITE (L3's old elimination-model may disagree — `genuine_wrong=0` is the hard gate; every
  flip needs its compiler-correctness argument documented).

**T3 — preprocessor foundations:**
- Union-read TRUE for objects/routines/globals (`is_preproc_wrapper` `lower/mod.rs:57-61` + the generic recursion) —
  `#if UNDEFINED procedure Foo() #endif` lands in `ObjectDecl.routines` (superset ⇒ sound for absence, unsound for
  resolution-confidence — a resolved route may target dead-branch code; DOCUMENT this honestly).
- The 4 fixtures (per the grounding): (1) the base union-read pin (al-syntax lowering unit); (2) both-arms `#if A
  procedure Foo #else procedure Foo #endif` → two distinct RoutineDecls (interacts with sig_fp/dedup — same-sig arms
  collapse-unmarked, different-sig arms distinct); (3) the `#if`-wrapped PROPERTY negative-control pin (SourceTable in
  `#if` currently dropped — feeds implicit_rec_table_id, a real narrow completeness gap: FIX the flat loop
  (`lower/mod.rs:168-173`) to descend preproc wrappers, or pin-and-defer with a dated note — prefer FIX, it's the same
  descend pattern used everywhere else); (4) same decision for the `implements` flat loop (`:200-222`).
- `ParseStatus::Clean` gate: zero consultation under `src/program` today. Decide: (a) thread a per-file
  `parse_status` into the snapshot/graph and DECLINE absence-adjacent claims (and/or count Recovered files in a
  diagnostic); minimum honest bar = a surfaced diagnostic count + a doc note; prefer the diagnostic now, the full gate
  when a consumer needs it. Fixture: an unbalanced `#if` forcing Recovered → assert the diagnostic fires.

## Global Constraints

- `rustfmt <file>` per-file — NEVER `cargo fmt`. Stage only named files — NEVER `git add -A`. `CHANGELOG.md` per task.
  CI gates: `cargo clippy --release --all-features -- -D warnings` (NO `--tests`), `cargo fmt --check`,
  `cargo test --workspace` (no CDO_WS, green), the FULL CDO harness per task.
- **Soundness cardinal:** T1 adds catalog members that EXIST unconditionally (never argument-validated — same as every
  other catalog method; document the rationale correction). T2 picks ONLY on exact-normalized full-coverage matches —
  any untyped position, 0-or->1 compatible, degraded set, SymbolOnly tier → no pick. Every CDO flip adjudicated with a
  compiler-correctness argument. `genuine_wrong=0` hard.
- **Correctness over compatibility (user directive):** rebaseline tests/pins/goldens where the new behavior is
  verifiably right (the ws_overload_collision flip; the ambiguousResolved pins; fresh-bucket movements).
- Determinism; additive-only export changes; ratchets DOWN with dated notes.
- **Out of scope:** ProvenAbsent machinery (the 7-site prototype documented for a future plan); ABI-tier arg dispatch;
  implicit conversions; Enum::Value/call-result/field arg typing (deferred increments); the eCandidates workspace-code
  bug itself (it's Continia's code, not the engine's).

## Tasks

### Task 1: Page/Report instance-catalog completion (the 18-site fix)
**Files:** `resolver.rs` (`is_metadata_sensitive_instance_method` removal/narrowing + the catalog entries),
`member_catalog.rs` (if the names live there); fixtures + gates.
- [ ] Step 1: failing fixtures — `PageVar.SetTableView(Rec)` / `.SetRecord` / `.GetRecord` / `.SetSelectionFilter` /
  `.SaveRecord` + `ReportVar.SetTableView` on typed Page/Report receivers → Catalog resolves (today: Unknown
  MemberNotFound); CONTROL: a genuinely-absent member still Unknown; the CurrPage.Part.Page.GetRecord subpage shape.
- [ ] Step 2: run — fail. Step 3: implement (delete/narrow the exclusion; per-entry provenance comments citing MS Learn
  + the L3 catalog precedent; document the rationale correction — existence ≠ argument-validation).
- [ ] Step 4: FULL CDO harness — expect `MemberNotFound` 25→7, `unknown` 95→~77, rate ~0.43%; **the golden interaction
  check FIRST**: these 18 move toward `matches` (L3 resolves them) — verify per-site targets agree (any disagreement →
  investigate before landing); `fresh_missing` may drop; `genuine_wrong=0`. EXHAUSTIVE adjudication of all 18 flips.
  Ratchets re-derived, dated. The 7 eCandidates sites: assert they REMAIN Unknown + document as the ProvenAbsent
  prototype population. Step 5: gates + commit
  `fix(resolve): complete the Page/Report instance catalog — SetTableView/SetRecord/GetRecord-class members exist (Task 1)`.

### Task 2: Fail-closed arg-type dispatch (source tier, literals + declared vars)
**Files:** `extract.rs` (args on RawSiteV2), `full.rs` (precompute arg_types), `resolver.rs`/`receiver.rs` (threading +
the pick), `sig_fp.rs` (pub(crate) normalize); the orphaned fixtures wired; gates.
- [ ] Step 1: failing fixtures — wire `ws-overload-arg-type` (declared `InStream` var picks the InStream overload),
  `-arg-pos2` (the discriminating position isn't the first), `-negatives` (Variant arg / same-family scalars / untyped
  positions → NO pick, stays AmbiguousResolved); the literal case (the `ws-overload-collision` `Resolve(5)` flip →
  Resolved to the Integer overload — rebaselined as strengthening); deferred-increment guards (`-enum-discriminator`,
  `-field-discriminator`, `-callexpr-discriminator` assert NOT-yet-flipped); a SymbolOnly-tier control (no pick).
- [ ] Step 2: run — fail. Step 3: implement (the plumbing; literal+var typing helper; the exact-match pick after
  prevalidation; tier gate; untyped-never-eliminates).
- [ ] Step 4: FULL CDO harness — record exactly WHICH of the 56 flip (expect a minority; adjudicate EVERY flip per-site:
  the argument shapes, the picked overload vs what the compiler would choose, the fresh/L3 bucket movement);
  `ambiguousResolved` pins re-derived; `genuine_wrong=0` hard. Step 5: gates + commit
  `feat(resolve): fail-closed arg-type overload dispatch — exact-match literals + declared vars, source tier (Task 2)`.

### Task 3: Preprocessor foundations (fixtures + the two flat-loop fixes + the ParseStatus diagnostic)
**Files:** `crates/al-syntax/src/lower/mod.rs` (the properties + implements descend fixes), al-syntax lowering tests,
snapshot/graph (the Recovered-count diagnostic), gates.
- [ ] Step 1: failing fixtures — the 4 designs (base union-read pin; both-arms distinct RoutineDecls incl. the
  same-sig-collapse/different-sig-distinct dedup interplay; the `#if`-wrapped SourceTable property NOW captured (the
  fix) + a control; the `#if`-wrapped implements clause NOW captured; the unbalanced-`#if` Recovered diagnostic).
- [ ] Step 2: run — fail. Step 3: implement (descend `is_preproc_wrapper` in the two flat loops — the established
  pattern; the Recovered-file diagnostic count surfaced on ProgramReport/aldump additively; doc the
  superset-semantics honestly where absence-adjacent reasoning lives). NOTE the al-syntax crate boundary: run the FULL
  suite (L2 features may shift — inspect any golden movement; additive-only expectations).
- [ ] Step 4: FULL CDO harness — expect byte-identical-or-adjudicated (a `#if`-wrapped SourceTable/implements on CDO
  could change resolution — adjudicate any movement as a correctness fix); `genuine_wrong=0`. Step 5: gates + commit
  `fix(al-syntax): preproc union-read for properties + implements; ParseStatus diagnostic; union-read pinned (Task 3)`.

### Task 4: Measure + close
- [ ] Full re-measure (all gates); adjudication sign-off (18 + the T2 flips + any T3 movement == the deltas); ratchets
  at the floors, dated; CHANGELOG (the arc: the falsified-ProvenAbsent story told honestly — the 13 were an engine gap,
  not absences; the catalog fix; the dispatch increment; the preproc pins; DEFERRED: ProvenAbsent for the 7 (the real
  population), the deferred arg-typing increments, ABI arg dispatch, implicit conversions); charter memory + MEMORY.md.
  Commit: `docs(resolve): catalog completion + arg-type dispatch + preproc foundations — real-unknown 0.52%→X% (Task 4)`.

## Roadmap — beyond this plan
ProvenAbsent prototyped against the 7 eCandidates sites (the verified-real absence population; needs the new
outcome/completeness machinery per plan-8's grounding); the deferred arg-typing increments (Enum::Value, call-result,
Rec.Field args); ABI param-type retention (unlocks SymbolOnly dispatch); implicit-conversion modeling (compiler-backed);
the full ParseStatus gate; deeper CompoundReceiver chains; protected Variables[]; Sender param-TYPE.
