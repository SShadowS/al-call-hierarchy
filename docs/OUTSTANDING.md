# Outstanding items

Living checklist — tick items (`- [x]` + landing commit/date) as they land; add new
items as they surface. Rewritten clean 2026-07-17 (post preflight-fresh-coverage +
outstanding-sweep runs); the full histories of completed arcs live in the Archive at
the bottom, CHANGELOG, and git log.

## Open — needs the user

- [x] `git stash drop` leftover stashes — DONE 2026-07-17: user ran `git stash clear`
  (26 accumulated scratch stashes from merged arcs, all superseded; verified 0 remain)
- [ ] Decide `/triage-wave` command sharing: `.claude/` is gitignored so it is
  local-only today — force-add `.claude/commands/triage-wave.md` to share, or keep personal
- [x] **d61/d62/d64 validation** — DONE 2026-07-17 (`f3f5c85`). Corpus: Microsoft
  System App + Base App 28.0 embedded source extracted from DO's `.alpackages`
  (9.3k real files). d62: 9 findings triaged (1 real, 8 FP) → structural
  branch-exclusivity class ROOT-CAUSE FIXED via statement_tree (9→4; the
  `if Success then Log else Error` idiom no longer flags); 3 residual semantic FP
  classes documented → stays opt-in. d64: first population (8 API pages) → only FP
  class (SourceTableTemporary) fixed, 2→0 with honest skips → stays opt-in (no TP
  yet). d61: 7,367 real candidates, 0 emissions, guards hold (caveat: sliced corpus
  may hide cross-slice event pairs) → stays opt-in. Promotion wake for all three:
  a triaged true-positive population

## Open — buildable backlog (no blocker, pick up any time)

- [ ] **tree-sitter-al quirks list** (low priority — engine is insulated by the
  lowerer, workarounds in place): spurious `left`/`operator`/`right` field pollution,
  `case_else_branch` inconsistency (see memory `tree-sitter-al-grammar-issues`)
- [ ] **Multi-root LSP workspaces** (user decision 2026-07-13: shipped single-root +
  tracked follow-up): per-root ServerState map + URI→root routing; design recorded in
  `server.rs`'s `primary_workspace_root` doc
- [ ] **Snapshot-scoped LineTable cache** — the remaining linear per-save cost on the
  LSP hot path (deep-review T3 follow-up)
- [ ] **Unicode-fold moat task** — 212 `to_ascii_lowercase` sites in `src/program/`;
  the one legitimate future north-star-SHA-mover (case-folding correctness for
  non-ASCII identifiers)
- [ ] **r3a4 source-bearing-dep pin hardening** (final-review nit, one line): assert the
  primary's `declared_deps` contains the dep GUID in
  `fresh_coverage_source_bearing_dep_not_opaque`, so a future fixture edit can't
  silently vacate the pin

## Parked — deferred WITH evidence; do NOT start without the wake condition

- [ ] **Preflight shared parse** — measured 2026-07-17: duplicated work is the PRIMARY
  app's parse only (deps parse once in the fresh pass); on DO that's 407 files of a
  dep-dominated 4.8 s resolve → sub-second saving. Live BOM divergence (DO has 4
  BOM-carrying `.al` files; snapshot keeps BOM, L3 strips) makes naive sharing
  behavior-changing. Investigation: `.superpowers/sdd/shared-parse-investigation.md`.
  **Wake:** analyze latency becomes user-facing pain, dep-parse caching lands, or BOM
  handling gets unified anyway
- [ ] **FreshCoverage ABI-error / missing-dep reporting** (+ serde-default-empty
  exemption hardening) — population-less on DO (0 ingest failures, 0 declared-but-
  missing; real ingest failures already surface as could-not-verify). **Wake:** the
  first real failing-ingest or missing-declared-dep population, or a SymbolReference
  emitter shape change
- [ ] **Number-less object identity collision (engine-wide)** — `o.id.unwrap_or(0)`
  (`src/engine/l2/l2_workspace.rs:355/414/593`) gives every Interface/ControlAddIn in
  an app the id `{guid}/{type}/0`. Harness symptom fixed; harm latent (DO: 5
  interfaces share one id, zero shared routine names → no routine-id collapse
  observed). Fix is a stable-id earthquake (fingerprints/baselines/digests/cache).
  **Wake:** two same-app number-less objects sharing a routine name, a misattributed
  production finding on an interface, or the next planned stable-id break (piggyback)

## Parked — call-graph roadmap (doctrine-deferred, population-less)

- [ ] ProvenAbsent — wake: a real proven-absence population (MemberNotFound is 0)
- [ ] Implicit conversions — wake: nonzero `ambiguousResolved` (currently 0)
- [ ] Full ParseStatus gate — wake: the first absence-claiming consumer
- [ ] Protected `Variables[]` — wake: an extension routine consuming a base protected var
- [ ] Preproc-symbol fidelity — wake: a real consumer
- [ ] Sender param-TYPE drift analysis — wake: a version-drifted-closure corpus

## Separate track

- BC-Brain — its own product backlog (`SShadowS/bc-brain`), never mixed into this list.

---

## Archive — completed (compressed; details in CHANGELOG + git log)

2026-07-17, outstanding-sweep run:
- [x] Push master to origin (113 commits, `e6b1283..d695392`; then continuously)
- [x] Differential-harness identity keying + wrong IEmpty fingerprint golden
  (`fix/outstanding-test-bugs`; "flaky" claim falsified — was deterministic-wrong)
- [x] gate_sarif regen-mode anti-degenerate bypass (`819790d`)
- [x] condition_references consumer audit — CLEAN, no consumer bitten
  (`.superpowers/sdd/condref-audit-report.md`)
- [x] d56 re-promotion OPT-IN → DEFAULT via keyRemappedClone analysis (`752a496`;
  DO: 0 findings, both real key-remap sites verified excluded)
- [x] MERGE-TIME CRLF re-materialization on master (552 files; detection law: use
  `file`/`od`, never grep — MSYS grep strips CR)
- [x] Stale-section corrections: deep-review T0-T4 ALL merged long ago (T2 `542740e`,
  T3 = the LSP-migration arc incl. legacy-pipeline deletion, T4 `d99c65e`); both
  Recovered-parse grammar defects fixed at grammar-defects-and-repin
  (`recoveredFiles` re-measured 0 on CDO)

2026-07-17, preflight-fresh-coverage arc (`d14cf84`):
- [x] §1 preflight fix — analyze preflight re-keyed to the fresh resolver
  (FreshCoverage + could-not-verify state + fail-closed hole + empty-ABI exemption);
  DO warning gone, totalFindings 2307 exact, north-star SHA byte-identical

2026-07-16/17, BCQuality detector wave (`8bb9756`):
- [x] 13 detectors d52–d64 + `bcquality` preset; FP triage on DO; root-cause fixes for
  d53/d56/d60/d63 (only d56 was temporarily opt-in, since re-promoted)
