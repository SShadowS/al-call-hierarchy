# Roadmap-closure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

> Status: **v2.1** (round 2: both GO-WITH-CHANGES → closers folded; the body reconciled below. BOTH addenda sections
> BINDING).

## Round-2 closers (BINDING)

- **T4 option (b) STRUCK (gemini):** scanner-EXCLUSIVE ownership of the preproc-open/close token space is the ONLY
  permitted design — a scanner/literal split is a GLR trap categorically (overlapping lexer states fork; the epoch
  history proves the class). If recovery constraints make scanner-exclusive infeasible, the task STOPS and reports —
  no fallback to a split. Recovery-fallback needs, if any, are built INTO scanner.c's state machine.
- **T1's ArityMismatch policy gets an `al compile` PROBE (gemini):** fixtures prove OUR behavior, not AL's — before
  the Report wrapper lands, probe the real compiler (the grammar repo's al-compile methodology) on a
  ReportExtension-procedure arity mismatch and record the diagnostic class; the wrapper's policy cites the probe.
- **The QueryExtension probe covers every code-bearing member shape** (procedures + triggers + any documented form),
  not just `procedure`; the retirement wording cites the probe results per shape.
- **Executable gates (gpt):** Task 0 added (freeze the baseline); T1's first checkbox = the behavioral inventory;
  T3's probe-before-wording checkbox; T4's token-ownership decision is a named acceptance artifact in the report.
- **T5 counts are computed AFTER the probes** (not hard-coded now); BC-Brain sits under a separate product-backlog
  heading, never inside the doctrine-deferred call-graph list.
- The body below is reconciled to the addenda (the superseded text corrected in place — no reliance on
  "supersedes" alone).

## Round-1 review addenda (BINDING)

**T2 — semantic ABI identity behind a structural guard (both CRITICAL):**
- Retain SEMANTIC identity, not just text: `AbiParamRetained{type_text, is_var, subtype_id, subtype_raw_name,
  subtype_tag}` (the tuple `AbiParameter` already carries). Canonicalization goes through an ABI-AWARE route: the
  subtype resolves via the SAME semantic object identity used for source params (`Record 36` == `Record "Customer"`
  iff the same resolved object — the resolve_object_ref precedent); a degraded/unresolvable ABI text or subtype →
  that param is UNTYPED → the candidate is metadata-incomplete → the CALL degrades. Fixtures: Record-id-vs-name
  equality, unresolved subtype, enum/interface subtypes, and at least one REAL generated SymbolReference shape (from
  an actual .app), not only hand-authored text.
- **The structural guard:** `AbiParams` is an ENUM — `Complete(Vec<AbiParamRetained>) | Missing | CollapsedUntrusted`
  — populated at ingestion (`abi_overload_collapsed` ⇒ `CollapsedUntrusted`); `candidate_param_infos_abi` accepts
  ONLY `Complete`. Reading collapsed params is impossible by type, not convention. Fixture: a collapsed set with
  seemingly-discriminating params still declines.
- Mixed-set fixtures explicit: complete-BodyMap + INCOMPLETE-ABI candidate → the whole call degrades (the
  no-filtering rule); complete-BodyMap + complete-ABI → picks only when ALL candidates are complete-comparable.

**T4 — the scanner/literal race is the design gate (both CRITICAL):**
- The design note (BEFORE code) must resolve the token-ownership question: if the external scanner matches spaced
  forms while grammar literals (`'#if'` etc.) remain, unspaced input can dual-match → the SAME instability class as
  the reverted attempt. Options: (a) scanner-EXCLUSIVE ownership of all preproc-open/close variants (remove the
  grammar literals — but FIRST understand why they exist: the error-recovery fallback when the scanner declines;
  removing them must not break recovery — prove with the recovery negatives) or (b) a proven-non-racing split. The
  choice is made from reading scanner.c + the generated parser states, then PROVEN empirically.
- Consume order: the scanner consumes `#` FIRST, then tests/consumes horizontal whitespace — never leading-whitespace
  consumption before `#` (recovery-boundary protection). No partial-consume-on-reject where avoidable; document the
  reject paths.
- **The empirical bar (raised from last time):** the ≥5 clean-cache `tree-sitter test` loop AND a full BC.History
  parse ×2 from clean cache comparing tree/error MANIFESTS (not one pass); unspaced `#if` trees byte-identical
  before/after (the tree-harness); recovery negatives: `#\nif`, `# ifx`, non-directive `# ...`, malformed nesting.
- Serialization: assert NO scanner serialized-state change (the 1-byte depth counter untouched); one
  incremental-parse-oriented nesting fixture with spaced forms.

**T1 — inventory before unification (both):**
- A PRE-REFACTOR behavioral inventory of BOTH functions (candidate collection, extension filtering, visibility rules,
  closure anchor, arity/access reason priority, tier handling, ambiguity ordering) goes in the report BEFORE the
  refactor; `ZeroMatchStrategy` must be shown to cover the ONLY intentional divergence. Zero-fixture-edits +
  byte-identical CDO are POSTCONDITIONS, not the proof.
- The Report wrapper's `PreserveArityMismatch` gets its rationale stated (a visible same-name wrong-arity routine
  surfaces ArityMismatch everywhere in this resolver — Page precedent; invisible/out-of-closure wrong-arity must NOT
  leak) + the 3 fixtures (visible wrong-arity, invisible wrong-arity, mixed base+extension wrong-arity).

**T3 — the retirements reworded honestly (both):**
- **Sender param-TYPE → DEFERRED-WITH-WAKE, not retired:** impossible in compile-valid AL under a CONSISTENT
  dependency closure; version-drifted closures (a shipped .app compiled against an older publisher) CAN present
  mismatches. Wake: a real corpus with stale/version-drifted symbol closures demanding drift analysis.
- **QueryExtension → retired NARROWLY with a verification gate:** `queryextension` EXISTS as an AL object type (the
  prior "nonexistent construct" wording was false); the claim is no CALLABLE ROUTINE MEMBERS in valid AL — verify via
  the grammar repo's `al compile` probe methodology (a queryextension with a procedure — expect rejection) and record
  the probe result; wake: the compiler/spec ever permitting callable queryextension members.

> **DATED CORRECTION (Task 5, 2026-07-05, append-only — the bullet above stays unedited).** The claim just above —
> "`queryextension` EXISTS as an AL object type... the prior 'nonexistent construct' wording was false" — was itself
> **FALSIFIED** by Task 3's mandatory pre-wording probe (`al.exe` v18.0.37.11445, CDO's `.alpackages` cache,
> platform/application `28.0.0.0`): 3/3 code-bearing shapes (bare, `+procedure`, `+trigger OnBeforeOpen()`) all
> reject identically with `AL0198: Expected one of the application object keywords (table, tableextension, page,
> pageextension, pagecustomization, profile, profileextension, codeunit, report, reportextension, xmlport, query,
> controladdin, dotnet, enum, enumextension, interface, permissionset, permissionsetextension, entitlement)` —
> `queryextension` is absent from the compiler's own enumerated keyword list, confirmed against a positive control
> (a bare `query` object in the same project compiles clean, exit 0). **The disposition reverts to the ORIGINAL,
> pre-round-2 wording: RETIRED (nonexistent construct)**, not "retired narrowly with a verification gate" as this
> addendum instructed. See `.superpowers/sdd/task-3-report.md` §1 for the full probe transcript and
> `CHANGELOG.md`'s `[Unreleased]` "Roadmap dispositions, probe-grounded (Task 3...)" entry for the recorded
> correction. Wake condition unchanged in spirit: a future AL compiler version ever adding a `queryextension` object
> keyword (re-probe before ever re-asserting either way).

**Cross-task (gpt I5/M1/M2):**
- **The frozen mechanical baseline:** BEFORE T1, capture one canonical CDO baseline artifact (engine SHA, grammar
  SHA, the harness command, all metrics, coverage totals, digests) into the report dir; every task compares against
  THAT artifact (scripted), not against the previous task's memory.
- The T5 counts recomputed after the rewordings; the product backlog (BC-Brain) listed SEPARATELY from
  doctrine-deferred call-graph items.
- The T4 no-push gate explicit: no grammar tag/push and no engine push until local grammar commit + local engine
  repin + gen-syntax zero-diff + engine suites + CDO byte-identical + BC.History ×2 manifest-compare ALL pass. Thirteenth arc (master `0b3945e`; CDO real-unknown 0.0000% [0/18108],
> ambiguousResolved 0, recoveredFiles 0, genuine_wrong 0; grammar v3.1.0 published). The grounding report (this
> session) is authoritative. THE ARC'S NATURE: machinery-completion + hygiene — the zero-metrics must stay EXACTLY
> zero throughout (any movement = STOP); the deliverables are fixture-proven machinery for standard-AL constructs CDO
> doesn't exercise, one grammar-limitation closure, and the roadmap's honest final state.
> PERMANENT LAW reminder: `.dependencies` folders are ordinary source — nothing may premise on the name.

**Goal:** Close every remaining buildable roadmap item (the Report/ReportExtension merge via scope-resolver
unification; ABI param-type retention lifting the SymbolOnly dispatch gate; the Step-4b WithState symmetry guard; the
spaced-`# if`/`# elif` scanner route in the grammar), retire the impossible items with recorded reasons, correct the
stale list entries append-only, and leave the roadmap containing ONLY doctrine-deferred items (population-less
taxonomy awaiting real corpora).

**Tech Stack:** Rust engine + the tree-sitter-al grammar (v3.2.0 at the end). FOREGROUND everything. Full CDO harness
per task (`CDO_WS="U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud"` + `ENFORCE_CDO_WS=1`, `--test-threads=1`).
Clippy `--all-targets` clean. Zero-metric strictness: NOTHING moves on CDO in any task (all machinery is
CDO-population-less by grounding; byte-identical harness runs are the acceptance bar).

## Key facts (verified on `0b3945e` / grammar `307dc39`; the grounding report is authoritative)

- **T1 (Report merge + unification):** `resolver.rs:2421` routes `Page` to `resolve_in_page_scope`, everything else
  (incl. Report) to bare `resolve_in_object` — the same pre-fix shape the Page gap had. `resolve_in_table_scope`
  (`:967-1064`) and `resolve_in_page_scope` (`:1155-1277`) are ~90% identical scaffold diverging only in the
  zero-arity-match branch (Table → access_excluded reason; Page → ArityMismatch-preserving forward). UNIFY:
  `resolve_in_extendable_scope(base, extensions, zero_match: ZeroMatchStrategy)` (~110 shared lines + two ~20-line
  wrappers), then `resolve_in_report_scope` (a third ~20-line wrapper, `PreserveArityMismatch`) +
  `report_extensions_of` (clone the `index.rs:210-229` population pattern + a ~10-line accessor; `extends_target` is
  already populated for ReportExtension — `node_extract.rs:545`). The unification is a REFACTOR of two live,
  heavily-fixture-covered functions — the PRE-REFACTOR BEHAVIORAL INVENTORY (addenda) is the proof instrument;
  zero fixture edits + byte-identical CDO are postconditions. The wrapper's ArityMismatch policy cites the
  al-compile probe (round-2 closer). QueryExtension: retired NARROWLY post-probe (a REAL AL object type; the claim
  is no callable routine members in valid AL — probe every code-bearing shape; wake condition recorded).
- **T2 (ABI param retention → SymbolOnly dispatch):** `AbiParameter` (`engine/deps/symbol_reference.rs:43-86`)
  ALREADY carries `type_text` + `is_var` + the subtype tuple — nothing new parses. `abi_ingest.rs:421-499` folds
  param types into sig_fp then discards (`param_sig_key: String::new()` :481) while `return_type`/`return_type_id`
  (:482-493) show the retention precedent. DESIGN (per the addenda): `RoutineNode.abi_params: AbiParams` — the
  STRUCTURAL enum `Complete(Vec<AbiParamRetained{type_text, is_var, subtype_id, subtype_raw_name, subtype_tag}>) |
  Missing | CollapsedUntrusted` (collapsed populated at ingestion; reads impossible by type);
  `candidate_param_infos_abi` accepts ONLY Complete and canonicalizes via the ABI-AWARE semantic route (subtype →
  the same resolved-object identity as source; degraded → untyped → the call degrades); the
  `resolver.rs:556` gate (`obj_tier != SymbolOnly` — the ONLY `candidate_param_infos` call site) lifts to
  BodyMap-then-ABI where metadata is complete (any candidate lacking retained params → the existing
  call-level-degradation rule declines — NO unknown-metadata candidate is ever filtered out). Memory: negligible
  (tens of MB at the 259k pre-dedup routine ceiling). Guards: var-mode fidelity is REAL (is_var carried) — the
  ByRef-exact rule applies; Variant/soft-family/collapse gates unchanged. Fixtures: a SYNTHETIC SymbolOnly overload
  corpus (an ABI-only app fixture — check how existing SymbolOnly fixtures are built, `ws-*` app.json w/o source?
  the abi fixtures precedent) proving: distinct-param-type ABI overloads pick; a var-mode ABI param eliminates a
  literal arg; a collapsed-marker set still declines; a missing-metadata candidate degrades the call.
- **T3 (symmetry + hygiene):** (a) Step 4b (`receiver.rs:1035-1049`) — add the identical
  `bare_ctx`/`NoWithProven` guard Step 3a has (`:915-926`); 2 fixtures (InsideWith/Unknown decline; NoWithProven
  preserves). (b) Append-only errata: the stale `CHANGELOG.md:1966-1967` "deliberately deferred" note (unquoted bare
  fields LANDED in the tenth arc — a dated correcting entry); the charter memory's list purges (dot-quoted +
  unquoted = DONE; Sender param-TYPE = RETIRED [a Sender-type mismatch is a compile error — structurally impossible
  population; EventFlow already fully disambiguates via the attribute triple]; QueryExtension = RETIRED
  [nonexistent construct]; protected `Variables[]` = DEFERRED-WITH-DESIGN [real declarations exist on CDO (3 pages),
  ZERO consuming extension sites; the 3-layer lift documented: VarDecl access-modifier field (grammar parses it,
  lowering drops it) → ObjectNode globals exposure → the scope-merge analog]). (c) Worktree cleanup: remove the
  stale diagnostic worktrees (`git worktree remove` — the `--force` one needs the user informed it's scratch-only;
  list first, remove the unambiguous ones, report).
- **T4 (the spaced-`# if`/`# elif` scanner route — grammar v3.2.0):** the documented-rejected limitation closes
  PROPERLY this time: the external scanner's `PREPROC_OPEN`/`PREPROC_CLOSE` (+ the `#else`-class if the scanner
  handles them — read scanner.c first) learn to CONSUME horizontal whitespace between `#` and the keyword as part of
  the token (`lexer->advance(lexer, false)` — never skip; never `isspace()` — `' '`/`'\t'` only), keeping the depth
  counter correct for spaced forms; token ownership per the round-2 closer: SCANNER-EXCLUSIVE over the
  preproc-open/close space (the grammar literals' recovery-fallback purpose understood FIRST; recovery needs built
  into scanner.c; infeasible → STOP, never a split). THE HARD LESSONS
  BINDING: the previous literal-variant attempt caused GLR non-determinism — the scanner route must NOT reintroduce
  grammar-level ambiguity (the scanner token and the literal fallback compete — understand the existing
  fallback-vs-scanner precedence FIRST; the `preproc_open` rule's structure); the honest-rejection fixtures FLIP to
  positives (spaced `# if` now parses, depth-correct — the nesting fixture from the review becomes the proof);
  cross-line negatives stay (`#\nif` must NOT match — the scanner consumes only horizontal); the STABILITY PROTOCOL
  (≥5 clean-cache `tree-sitter test` runs, identical counts; the split-begin repro md5-stable ×5); BC.History
  (15,358 — 0 errors + tree-harness: previously-valid trees byte-identical, the ONLY diffs = spaced-if files if any
  exist [expect zero — corpus-absent]); `tree-sitter test -u` discipline per the plan-12 rules. Version v3.2.0
  (additive), tag. THE INVERTED SEQUENCE (plan-12's rule): everything validates LOCALLY (grammar local commit →
  engine local repin → gen-syntax zero-diff [scanner.c changes don't touch node-types — verify] → full engine
  suites byte-identical) BEFORE any push; the publish pair rides the merge menu. The grammar CHANGELOG's
  "Not supported (rejected)" section gets its resolution appended (the scanner route landed — append-only).
- **Doctrine-deferred (recorded, NOT built):** ProvenAbsent (blueprint in the pageext plan's roadmap; awaits a real
  absence population + must consult recoveredFiles); implicit-conversion modeling (awaits an ambiguousResolved
  population); the full ParseStatus gate (awaits an absence-claiming consumer); protected Variables[] (above);
  preprocessor-symbol fidelity for embedded deps (`compilation.rs:26-32` — awaits a real consumer).

## Global Constraints

- `rustfmt <file>` per-file — NEVER `cargo fmt`. Stage only named files — NEVER `git add -A`. CHANGELOG per task
  (both repos where touched). Gates per task: clippy `--release --all-features --all-targets -- -D warnings`,
  `cargo fmt --check`, `cargo test --workspace` (al-sem differential goldens zero-divergence), the FULL CDO harness.
- **Zero-metric strictness (the arc's bar):** every task's CDO harness run is BYTE-IDENTICAL on all metrics
  (0/18108, ambiguousResolved 0, recoveredFiles 0, genuine_wrong 0, coverage totals, digests). Any movement = STOP
  and investigate. The T1 refactor additionally proves behavior-preservation via zero fixture edits.
- **Soundness:** T2's gate lift keeps every dispatch guard (call-level degradation, ByRef-exact w/ real is_var,
  Variant, soft-family, collapse-marker degrade); no unknown-metadata candidate filtered out. T4 must not
  reintroduce ambiguity — the stability protocol is the acceptance bar.
- Retirements/corrections are APPEND-ONLY errata (dated); memory updates at the capstone.
- Out of scope: everything doctrine-deferred; any new taxonomy.

## Tasks

### Task 0: Freeze the canonical CDO baseline
- [ ] Capture the baseline artifact (engine SHA, grammar SHA, the harness command, all metrics, coverage totals,
  digests) to `.superpowers/sdd/cdo-baseline-plan13.json` (or .md); every later task's harness run compares against
  THIS artifact, scripted. No code changes. (Folds into T1's commit if trivial.)

### Task 1: Scope-resolver unification + the Report/ReportExtension merge
**Files:** `resolver.rs` (the unification + wrapper), `index.rs` (`report_extensions_of`), fixtures.
- [ ] FIRST: the pre-refactor behavioral inventory of BOTH live functions (the addenda's dimension list) in the
  report; ZeroMatchStrategy shown to cover the only intentional divergence. THEN the al-compile probe (a
  ReportExtension-procedure arity mismatch — record the compiler's diagnostic class; the wrapper policy cites it).
- [ ] Failing fixtures (Report shapes): a base-Report-typed receiver calling a ReportExtension-declared
  same-app internal procedure → Resolved; different-app internal → declines; out-of-closure → invisible; two
  visible extensions → ambiguity (defensive); VISIBLE wrong-arity → ArityMismatch; INVISIBLE wrong-arity → no leak;
  mixed base+extension wrong-arity; base-only unchanged.
- [ ] The refactor: `resolve_in_extendable_scope` + `ZeroMatchStrategy`; Table/Page wrappers (existing fixtures
  UNTOUCHED and green — the behavior-preservation proof); the Report wrapper + index accessor; `resolver.rs:2421`
  routes Report too.
- [ ] FULL CDO harness byte-identical; workspace; gates. Commit:
  `feat(resolve): unify extendable-scope resolution; Report/ReportExtension routine merge (Task 1)`.

### Task 2: ABI param-type retention + the SymbolOnly dispatch lift
**Files:** `node_extract.rs`/`node.rs` (the field), `abi_ingest.rs` (population), `arg_dispatch.rs` (the ABI arm),
`resolver.rs` (the gate), synthetic SymbolOnly fixtures.
- [ ] Failing fixtures FIRST (the synthetic ABI corpus): distinct-param ABI overloads → pick; is_var eliminates a
  literal; collapsed set declines; missing-metadata candidate degrades the call; a mixed BodyMap+ABI set works.
- [ ] Implement per the design; FULL CDO harness byte-identical (CDO has no SymbolOnly routines — the lift is inert
  there by grounding); gates. Commit: `feat(resolve): retain ABI param types — SymbolOnly arg dispatch (Task 2)`.

### Task 3: WithState symmetry + roadmap hygiene + worktrees
- [ ] FIRST: the QueryExtension al-compile probe (EVERY code-bearing member shape — procedure, trigger, any
  documented form) — record results BEFORE any retirement wording.
- [ ] The Step-4b guard + 2 fixtures; the CHANGELOG errata entry; the dispositions recorded post-probe (QueryExtension
  narrow-retired-or-reclassified per the probe; Sender param-TYPE DEFERRED-WITH-WAKE [version-drift]; protected
  Variables[] deferred-with-design); the worktree cleanup (list → remove unambiguous → report the --force one).
- [ ] FULL CDO harness byte-identical; gates. Commit:
  `fix(resolve): Step-4b with-scope symmetry; roadmap retirements + errata (Task 3)`.

### Task 4: The spaced-`# if`/`# elif` scanner route (grammar v3.2.0, LOCAL until the menu)
**Files:** `tree-sitter-al/src/scanner.c` + `grammar.js` if needed (+ regenerated src/*), the fixture flips, grammar
CHANGELOG; the engine submodule gitlink + the harness note.
- [ ] Read scanner.c's PREPROC handling FIRST (the token set, the literal-fallback interplay, the depth machinery);
  design note in the report BEFORE coding. Then: consume-not-skip horizontal whitespace; the rejection fixtures flip
  to positives (incl. the NESTING case as the depth-correctness proof); cross-line negatives stay; `tree-sitter test`
  + the STABILITY PROTOCOL (≥5 clean-cache identical) + BC.History (0 errors; tree-harness byte-identical for
  previously-valid trees) + gen-syntax zero-diff on the engine + the FULL CDO harness byte-identical. LOCAL commits
  both repos (v3.2.0 + the engine repin); NO push — the publish pair rides the merge menu. Commit (grammar):
  `feat(scanner): whitespace-tolerant # if / # elif via consume-not-skip — depth-correct (v3.2.0)`.

### Task 5: Measure + close
- [ ] Full re-measure vs the T0 baseline (byte-identical everything); CHANGELOG capstone (the roadmap's final state:
  built / retired / deferred — the COUNTS COMPUTED from the actual probe outcomes, not pre-declared; each deferred
  item with its wake-up condition); charter memory + MEMORY.md (the roadmap list rewritten to the final honest
  state); the opus whole-branch → the merge menu (engine merge+push + grammar v3.2.0 publish pair together).
  Commit: `docs(resolve): roadmap closure — built/retired/deferred final state (Task 5)`.

## Roadmap — after this plan (doctrine-deferred call-graph items, each with its wake-up condition)
ProvenAbsent (wake: a real proven-absence population on any corpus); implicit conversions (wake: a nonzero
ambiguousResolved population); the full ParseStatus gate (wake: the first absence-claiming consumer); protected
Variables[] (wake: an extension routine consuming a base protected var in any corpus); preproc-symbol fidelity
(wake: a real consumer); Sender param-TYPE drift analysis (wake: a corpus with version-drifted symbol closures).

## Product backlog (separate from the call-graph roadmap)
BC-Brain integration work as the product demands.
