# Outstanding items

Living checklist — tick items as they land (`- [x]`), add landing commit/date.
Compiled 2026-07-17 after the BCQuality detector wave merge (`8bb9756`).
Ordering within a tier is the suggested attack order.

## Housekeeping (this week)

- [ ] Push `master` to origin (~45 commits ahead: BCQuality wave + dev-setup + preflight-fresh-coverage merges)
- [ ] `git stash drop stash@{0}` (accidental frozen-fixture renormalize; harmless, user runs it — safety net blocks the agent)
- [ ] Decide `/triage-wave` command sharing: `.claude/` is gitignored so it is local-only today — force-add `.claude/commands/triage-wave.md` or leave personal
- [ ] ws-interface-dispatch.golden.json IEmpty signatureFingerprint is HashMap-seed-flaky under REGEN (found during preflight arc; regen twice and diff to reproduce)
- [ ] gate_sarif_differential.rs regen path has a latent self-check bug (found during preflight arc — see .superpowers/sdd/task-4-report.md for details)
- [x] MERGE-TIME (preflight branch): master's working tree held CRLF copies of wave-era tests/r0-corpus .al files — DONE at merge d14cf84 (all 552 re-materialized via rm + git checkout-index; index renormalize settled status; master l2_ir + check-goldens green). Detection note: use `file`/`od` for CR checks, never grep (MSYS grep strips CR)

## BCQuality wave follow-ups (doc `2026-07-16-scanner-validation…` §6)

- [x] **§1 preflight fix:** `alsem analyze` preflight consumes legacy L3 coverage → misleading `analysis coverage degraded — 1045 unresolved callsite(s)` warning on DO. Spec approved: `docs/superpowers/specs/2026-07-17-preflight-fresh-coverage-design.md` (FreshCoverage status struct from the fresh resolver + could-not-verify preflight state + fail-closed hole fix). Landed `07512b2..af12890` on `feat/preflight-fresh-coverage` (capstone DO smoke clean: no warning, totalFindings 2307, north-star SHA `0a3b85bc…` unchanged)
- [ ] Preflight follow-up — shared parse: L3 assembly consuming `ProgramContext::parsed()` (halves the added analyze cost, kills the L3↔fresh TOCTOU)
- [ ] Preflight follow-up — report dependency ABI-ingestion errors + declared-but-missing deps in `FreshCoverage`; then re-strengthen the clean message; also harden the empty-ABI exemption against serde-default-empty parses of unrecognized SymbolReference shapes (a future emitter change could silently exempt a real dep)
- [ ] **d56 re-promotion:** add primary-key-field-reassignment analysis (clone whose PK/current-key field is reassigned before the write targets a DIFFERENT row — the MoveEmailLog shape), exclude those, re-promote d56 OPT-IN → DEFAULT
- [ ] **Validate d61/d62/d64 on real code:** emitted 0 on DO (fixture-proven only). Run `/triage-wave` on a corpus that exercises them before considering promotion
- [ ] Audit other `condition_references` consumers (e.g. d43 IsHandled-guard) for the paren/quoted-condition blind spot that bit d60; migrate to `statement_tree` where bitten

## Deep-review remediation arcs (standing arc, pre-wave)

- [ ] **T2** (next per plan; commit-before-gate law applies)
- [ ] **T3** — needs a brainstorm session BEFORE implementation
- [ ] **T4** (next per plan)
- T0+T1 merged; frozen CDO SHA `0a3b85bc…` re-verified 2026-07-16 ✓

## Grammar (tree-sitter-al — we own it)

- [ ] Fix the 2 real grammar defects surfaced by the `ParseStatus::Recovered` diagnostic (confined to dependency source; deferred at argtype-dispatch plan T3)
- [ ] Work the running quirks list: spurious `left`/`operator`/`right` field pollution, `case_else_branch` inconsistency (see memory `tree-sitter-al-grammar-issues`)

## Call-graph roadmap — PARKED (doctrine-deferred, do NOT start without the wake condition)

Each is population-less today; its wake condition is the ONLY trigger:

- [ ] ProvenAbsent — wake: a real proven-absence population (MemberNotFound is 0)
- [ ] Implicit conversions — wake: nonzero `ambiguousResolved` (currently 0)
- [ ] Full ParseStatus gate — wake: the first absence-claiming consumer
- [ ] Protected `Variables[]` — wake: an extension routine consuming a base protected var
- [ ] Preproc-symbol fidelity — wake: a real consumer
- [ ] Sender param-TYPE drift analysis — wake: a version-drifted-closure corpus

## Separate track

- BC-Brain — its own product backlog (`SShadowS/bc-brain`), never mixed into this list.
