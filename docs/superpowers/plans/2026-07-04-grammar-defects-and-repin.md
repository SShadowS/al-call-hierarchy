# Grammar-defects + repin Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

> Status: **v2.1** (round 2: both GO-WITH-CHANGES on body-text drift only — reconciled: hidden-tabledata + [ 	]* + the
> inverted sequence now IN the task bodies; the region-audit + scanner-hazard + CRLF negatives folded. The addenda below are BINDING and
> supersede conflicting task text).

## Round-1 review addenda (BINDING)

- **HORIZONTAL WHITESPACE ONLY (both CRITICAL):** `\s` includes `\n`/`\r` in the regex engine — a `#` at line-end
  would swallow `pragma` from the NEXT line (an extras token erasing real source). The pragma fix is
  `'(?i)#[ \t]*pragma[^\n\r]*'`; the `# if`/`# elif` fix (scanner or literal) skips ONLY `' '`/`'\t'` — never
  `isspace()`. Negative tests mandatory: `#\npragma`, `#\r\npragma`, `#\nif` must NOT match.
- **The tabledata rule is HIDDEN, not named (both CRITICAL):** a visible `tabledata_keyword` node would (a) change
  the tree shape of every VALID `tabledata_permission` (violating additive-only), (b) shift the RawKind vocab
  (gen-syntax churn), and (c) trip CI's unpinned-main freshness gate for any open engine PR the moment grammar main
  moves. Use a hidden `_tabledata_keyword` (underscore rule) or keep the inline token, surfacing in option-member
  position ONLY via the existing `alias(..., $.identifier)` route — the parse tree of an option member named
  TableData reads as a plain identifier; valid permission trees keep their EXACT shapes; the vocab does NOT move
  (VERIFY: gen-syntax produces zero diff — if it moves anyway, STOP and reassess). The broad
  keyword-as-identifier ACCEPTANCE stays (over-acceptance is idiomatic tree-sitter; the compiler is the strictness
  gate) — with the added tests: sole-member `OptionMembers = TableData;`, malformed permission-like input under
  OptionMembers (recovery must not silently parse the wrong construct), and a valid `tabledata_permission`
  regression control. If the fallback (option_member-only) is ever used, alias it as `identifier` (shape parity).
- **SEQUENCING INVERTED (gpt C3):** commit the grammar fix LOCALLY (do not push); point the engine's submodule at the
  local SHA; run gen-syntax (expect zero diff per the hidden route) + the FULL engine validation (workspace suite w/
  zero al-sem-differential divergence + the FULL CDO harness) against the LOCAL grammar FIRST. Only after everything
  is green: push grammar main + tag, then push the engine pin bump immediately (the CI hazard window shrinks to
  minutes, and a bad grammar never becomes public).
- **The -u refresh is REVIEWED hunk-by-hunk:** baseline `tree-sitter test -u` on a scratch copy BEFORE the grammar
  edit (isolating the stale-rename diffs), then apply the grammar fix and verify the 4 fixtures' final diffs are
  STRICTLY the `member_trigger_name`/`in_expression` rename classes — zero structural shifts anywhere else. Any
  unexpected hunk = STOP.
- **ZERO-METRIC STRICTNESS (gpt I5):** this plan may move ONLY `recoveredFiles` (8→0) and the grammar-repo test
  counts. ANY movement in real-unknown, ambiguousResolved, genuine_wrong, coverage totals, or semantic CDO output =
  STOP and investigate — no "adjudicated-additive" loophole (the un-Recovered content is resolution-inert per the
  prior span analysis; if coverage moves, that analysis was wrong and the plan halts).
- **BC.History honesty (gpt I3):** if the corpus is unavailable locally, the report states it verbatim ("BC.History
  not run; substitute coverage-limited") + runs the tree-harness before/after on every available corpus (CDO source
  at minimum). Sufficient to protect THIS engine repin; not claimed as ecosystem-equivalent.
- **The `# if` route matches the claim (gpt M1):** if the scanner route is taken → horizontal-only skip with boundary
  checks (full whitespace tolerance); if literal variants → the CHANGELOG scopes the claim to exactly the accepted
  forms (single-space), no over-claiming. Twelfth arc — infrastructure (master `5e4ee0c`, CDO real-unknown 0.0000%,
> ambiguousResolved 0, `recoveredFiles=8` pinned). TWO REPOS: the grammar submodule `tree-sitter-al/` (HEAD `f150581` =
> origin/main tip, `SShadowS/tree-sitter-al` — WE OWN IT; pushing to origin correct per the push policy) and the engine.
> The grounding report (this session) is authoritative; anchors below.

**Goal:** Fix the two Recovered-parse grammar defects (the `OptionMembers = TableData` keyword collision; the
`# pragma` whitespace intolerance) + the latent same-class `# if`/`# elif` gap, refresh the 4 stale test fixtures,
ship the grammar (tag + push), repin the engine, and retighten `recoveredFiles` 8→0 — with zero movement on the
zero-metrics (real-unknown 0.0000%, ambiguousResolved 0, `genuine_wrong=0`).

**Tech Stack:** tree-sitter grammar (grammar.js + regenerated parser.c/grammar.json/node-types.json — committed
TOGETHER per the grammar repo's own rule); Rust engine. FOREGROUND everything.

## Key facts (verified on `f150581` / engine `5e4ee0c`)

- **Defect 1 (`OptionMembers = TableData,...`):** `tabledata` is a bare inline `kw('tabledata')` in
  `tabledata_permission` (grammar.js:924-934); at the position after `=` both that token and `identifier` match
  "TableData" equal-length — the keyword wins the lexical tie, and `option_member`'s choice list (grammar.js:1034-1046)
  has no alternative accepting it → ERROR, first-position-only (later positions are already committed to
  `option_member_list`). The GLR conflict for this ambiguity is ALREADY declared (grammar.js:119). THE PRECEDENT:
  `table_keyword` has a dual role — keyword + listed in `keyword_as_identifier` (grammar.js:3924), and
  `option_member` already includes `alias($.keyword_as_identifier, $.identifier)` (:1041).
  **FIX (per the addenda — HIDDEN):** a hidden `_tabledata_keyword` (underscore rule) or the inline token kept,
  surfaced in option-member position ONLY via the existing alias-as-`identifier` route — NO visible new node kind
  (valid `tabledata_permission` trees keep their EXACT shapes; the RawKind vocab does NOT move — gen-syntax expected
  ZERO diff, STOP if it moves). FALLBACK if revalidation surfaces new conflicts: option_member-only acceptance,
  aliased as `identifier` (shape parity). The engine harness documents the confirmed minimal repro
  (`program_resolve_harness.rs:1353-1360`: `OptionMembers = TableData,Table,Report;` errors; `= Foo,TableData,Bar;`
  clean).
- **Defect 2 (`# pragma`):** `pragma: new RustRegex('(?i)#pragma[^\n\r]*')` (grammar.js:3174) — whitespace-intolerant.
  `preproc_else`/`preproc_endif` carry explicit `'# else'`/`'# endif'` variants (:2874-2876). CAUTION: the
  `preproc_region`/`preproc_endregion` `#\s*` pattern (:3176-3178) has the SAME cross-line bug class this plan must
  not copy — AUDIT those two in the same pass (convert to `[ \t]*` with `#\nregion` negatives, or record as scoped-out
  debt with the reason; stop citing them as a safe precedent). Legality: the file shipped in a compiled AppSource app
  → the real compiler accepts the spaced form. **FIX (per the addenda):** `'(?i)#[ \t]*pragma[^\n\r]*'` — HORIZONTAL
  whitespace only; pragma is a pure `extras` token (:88), zero GLR involvement.
- **The latent same-class gap (preventive fold-in):** `preproc_if`/`preproc_elif` (grammar.js:2829-2872 + the external
  scanner `src/scanner.c:158-166` — `PREPROC_OPEN` advances past `#` then immediately `read_keyword_ci("if")`, no
  whitespace skip) have NO `# if`/`# elif` tolerance. No corpus instance yet — fix preventively. **The
  LITERAL-VARIANT route is RECOMMENDED** (mirror `preproc_else`): the scanner route has a token-splitting hazard
  (mid-token whitespace must be consumed via `lexer->advance(lexer, false)` — part of the token — never skipped via
  `advance(_, true)`; getting it wrong splits the token). If the scanner route is taken anyway: horizontal-only,
  consume-not-skip, and `src/scanner.c` goes in the commit file list. The CHANGELOG scopes the claim to exactly the
  accepted forms. Negatives: `#\nif`, `#\r\nif`, `#\nelif`, `#\r\nelif`; positives per the chosen route's real
  tolerance (tab/multi-space only if the route genuinely accepts them).
- **4 stale `tree-sitter test` fixtures** (1444/1448 at HEAD; the 4 predate the `member_trigger_name` +
  `in_expression`-case-pattern fixes; verified no ERROR/MISSING in their actual trees): refresh via `tree-sitter test
  -u` (the repo's own rule permits -u only when no ERROR/MISSING — satisfied).
- **Validation:** the grammar repo's gate is BC.History (15,358 files) via `./validate-grammar.sh --full` +
  `tools/tree-harness.sh` (byte-identical tree-diff — the right tool to prove blast-radius containment). The corpus is
  EXTERNAL — check the usual locations (ask the ledger/user paths: try `U:/Git/BC.History` and siblings); if absent
  locally, document that CI-side/engine-side gates carry the validation: the engine's al-sem differential goldens
  (zero divergences = behavior-preserving per CLAUDE.md) + the full CDO harness + `tree-sitter test` 1448/1448.
- **Pin mechanics (no contradiction):** local dev = a genuine submodule pin (bump-commit convention, e.g. `626db43`);
  CI = an independent unpinned checkout of grammar `main` + the "Generated vocab is fresh" gate (`cargo run -p xtask --
  gen-syntax` then `git diff --exit-code`). The moment the grammar lands on main, CI builds against it regardless of
  the engine pin — hence **THE INVERTED SEQUENCE (binding):** everything validates LOCALLY first (grammar committed
  local-only, engine repinned to the local SHA, gen-syntax zero-diff verified, the FULL engine suites green) — and
  only then the publish pair: push grammar main + tag, push the engine bump immediately after (the hazard window
  shrinks to minutes; a bad grammar never becomes public).
- **Engine effects:** `recoveredFiles` 8→0 (the pinned list at `program_resolve_harness.rs:1405-1416` is the COMPLETE
  Recovered set — 4 files ×2 pre-dedup scopes); the un-Recovered content is resolution-inert (3 MS System tables'
  option fields + 1 pragma line — per the plan-10 review's span verification); expect real-unknown 0.0000% and
  ambiguousResolved 0 UNCHANGED, coverage possibly +0 or small-additive (adjudicate any movement). Version bump:
  additive-only → minor (3.0.1 → 3.1.0 per the v2.5.2→v2.6.0 precedent), tag.

## Global Constraints

- Grammar repo: grammar.js + ALL regenerated artifacts (parser.c/grammar.json/node-types.json) in ONE commit; the
  commit message carries the validation result (`[BC.History: N errors]` if the corpus is available, else the
  substitute gates run); `tree-sitter test` 1448/1448 before push; tag `v3.1.0`; push to origin main (SShadowS-owned).
- Engine repo: the submodule bump commit per convention; `gen-syntax` regen committed (the CI freshness gate); the
  `expected_recovered` ratchet retightened to EMPTY with a dated note; `rustfmt <file>`; stage only named files;
  clippy `--all-targets`; `cargo fmt --check`; `cargo test --workspace` (the al-sem differential goldens MUST show
  zero divergence — any divergence = STOP and investigate, the fix should be additive-only); the FULL CDO harness
  (`CDO_WS="U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud"` + `ENFORCE_CDO_WS=1`, `--test-threads=1`) — the
  zero-metrics byte-identical-or-adjudicated; `genuine_wrong=0`. CHANGELOG both repos.
- **Soundness:** the grammar fixes must be ADDITIVE-ONLY (previously-ERROR input becomes valid; already-valid trees
  keep their exact shapes — the tree-harness/differential goldens prove it). Any shape change to an already-valid
  tree = STOP.
- Out of scope: any other grammar refactor; the .dependencies double-include; ProvenAbsent.

## Tasks (THE INVERTED SEQUENCE — T1 is entirely LOCAL, T2 publishes)

### Task 1: The grammar fixes + LOCAL engine validation (NO PUSH anywhere)
**Files:** `tree-sitter-al/grammar.js` (+ regenerated src/*; + `src/scanner.c` IF the scanner route), corpus tests,
the 4 stale fixtures, grammar CHANGELOG; the engine submodule gitlink (local), `crates/al-syntax/src/raw/generated/*`
(verify-only), `tests/program_resolve_harness.rs` (the recoveredFiles ratchet).
- [ ] BASELINE the 4 stale fixtures first: `tree-sitter test -u` on the UNMODIFIED grammar (isolating the
  member_trigger_name/in_expression rename diffs; review each hunk); keep as a separate commit-in-waiting.
- [ ] Corpus tests (failing): `OptionMembers = TableData,Table,Report;` + sole-member `OptionMembers = TableData;` +
  mid-list control + a valid `tabledata_permission` regression control + malformed-permission-like-under-OptionMembers
  recovery; `# pragma warning disable X` + `#pragma` control + the cross-line NEGATIVES (`#\npragma`, `#\r\npragma`);
  `# if`/`# elif`/`# endif` per the chosen route + the cross-line negatives (`#\nif`, `#\r\nif`, `#\nelif`,
  `#\r\nelif`); the `#\s*region` audit outcome's tests.
- [ ] Implement per the BINDING addenda (hidden tabledata; `[ \t]*`; the literal-variant-recommended `# if` route);
  `tree-sitter generate`; `tree-sitter test` ALL green. COMMIT LOCALLY on a grammar branch — do NOT push.
- [ ] Validation: BC.History if locally available (state honestly if not) + `tools/tree-harness.sh` before/after on
  every available corpus (CDO source minimum) — blast radius = exactly the affected constructs, zero shape changes to
  previously-valid trees.
- [ ] LOCAL engine validation: point the submodule at the local grammar SHA; `cargo run -p xtask -- gen-syntax` →
  assert a STRICTLY EMPTY diff (the hidden route — any movement = STOP); retighten `expected_recovered` to empty
  (dated); `cargo test --workspace` (al-sem differential goldens ZERO divergence — any = STOP) + the FULL CDO harness:
  recoveredFiles=0 and NOTHING ELSE MOVES (real-unknown 0/18108 exact, ambiguousResolved=0, genuine_wrong=0, coverage
  totals identical — any other movement = STOP, no adjudication loophole). Commit the engine changes locally.

### Task 2: The publish pair + close
- [ ] Re-verify both local commits' gates one final time; then: push the grammar branch → merge/land on origin main +
  tag `v3.1.0` (commit message carries the validation result: `[BC.History: …]` or the honest substitution note);
  IMMEDIATELY push the engine commit (the submodule bump now pointing at the public SHA — re-point if the SHA changed
  via rebase/merge, re-run gen-syntax zero-diff + the metric gate once more if so).
- [ ] CHANGELOG both repos (the find→fix loop closed: the recoveredFiles diagnostic found them, this plan fixed them
  at the grammar source; the `#\s*region` audit outcome; the route's exact tolerance claims); the grammar-issues
  memory updated (the 2 defects → FIXED; the preventive `# if`; the region audit). Grammar commit:
  `fix(grammar): TableData as first option member; whitespace-tolerant # pragma / # if / # elif (v3.1.0)`. Engine
  commit: `chore(grammar): bump tree-sitter-al to v3.1.0 — Recovered-parse defects fixed, recoveredFiles 8→0 (Task 2)`.

## Roadmap — beyond this plan
The .dependencies double-include root cause; ProvenAbsent (blueprint recorded); ABI param retention;
Report/ReportExtension merge; implicit conversions; Step-4b WithState symmetry; protected Variables[]; Sender
param-TYPE; upstreaming consideration for the grammar fixes.
