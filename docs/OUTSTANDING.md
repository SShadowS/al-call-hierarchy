# Outstanding items

Living checklist — tick items as they land (`- [x]`), add landing commit/date.
Compiled 2026-07-17 after the BCQuality detector wave merge (`8bb9756`).
Ordering within a tier is the suggested attack order.

## Housekeeping (this week)

- [x] Push `master` to origin — DONE 2026-07-17 (`e6b1283..d695392`, 113 commits; secrets scan clean; Continia refs consistent with what origin already exposes)
- [ ] `git stash drop stash@{0}` (accidental frozen-fixture renormalize; harmless, user runs it — safety net blocks the agent)
- [ ] Decide `/triage-wave` command sharing: `.claude/` is gitignored so it is local-only today — force-add `.claude/commands/triage-wave.md` or leave personal
- [x] ws-interface-dispatch.golden.json IEmpty signatureFingerprint "flaky under REGEN" — FALSIFIED as HashMap-seed flakiness (6 fresh-process regen loops, before AND after the fix, were 100% deterministic both times). Root cause was `tests/differential.rs`'s `diff_snapshots` keying objects by `stableObjectId` alone, which collides for any two `Interface`/`ControlAddIn` objects in one app (both synthesize `objectNumber = 0`) — silently dropping all but the last-sorted colliding entry, so `IEmpty`'s stale/wrong golden fingerprint (a copy of `IProcessor`'s) was never actually compared. Fixed by keying on `(stableObjectId, name)`; golden rebaselined (only that one field moved) — DONE (fix/outstanding-test-bugs)
- [x] gate_sarif_differential.rs regen path had a latent self-check bug (regen mode bypassed the code-flows anti-degenerate assertion via `continue`) — DONE 819790d (fix/outstanding-test-bugs); both modes green, regen byte-identical
- [x] MERGE-TIME (preflight branch): master's working tree held CRLF copies of wave-era tests/r0-corpus .al files — DONE at merge d14cf84 (all 552 re-materialized via rm + git checkout-index; index renormalize settled status; master l2_ir + check-goldens green). Detection note: use `file`/`od` for CR checks, never grep (MSYS grep strips CR)

## BCQuality wave follow-ups (doc `2026-07-16-scanner-validation…` §6)

- [x] **§1 preflight fix:** `alsem analyze` preflight consumes legacy L3 coverage → misleading `analysis coverage degraded — 1045 unresolved callsite(s)` warning on DO. Spec approved: `docs/superpowers/specs/2026-07-17-preflight-fresh-coverage-design.md` (FreshCoverage status struct from the fresh resolver + could-not-verify preflight state + fail-closed hole fix). Landed `07512b2..af12890` on `feat/preflight-fresh-coverage` (capstone DO smoke clean: no warning, totalFindings 2307, north-star SHA `0a3b85bc…` unchanged)
- [ ] Preflight follow-up — shared parse: L3 assembly consuming `ProgramContext::parsed()` (halves the added analyze cost, kills the L3↔fresh TOCTOU)
- [ ] Preflight follow-up — report dependency ABI-ingestion errors + declared-but-missing deps in `FreshCoverage`; then re-strengthen the clean message; also harden the empty-ABI exemption against serde-default-empty parses of unrecognized SymbolReference shapes (a future emitter change could silently exempt a real dep)
- [ ] **d56 re-promotion:** add primary-key-field-reassignment analysis (clone whose PK/current-key field is reassigned before the write targets a DIFFERENT row — the MoveEmailLog shape), exclude those, re-promote d56 OPT-IN → DEFAULT
- [ ] **Validate d61/d62/d64 on real code:** emitted 0 on DO (fixture-proven only). Run `/triage-wave` on a corpus that exercises them before considering promotion
- [x] Audit other `condition_references` consumers (e.g. d43 IsHandled-guard) for the paren/quoted-condition blind spot that bit d60; migrate to `statement_tree` where bitten — AUDITED CLEAN 2026-07-17: ground truth is narrower than described (`Parenthesized` recurses fine and always has; the real gap is only a quoted-member-field leaf, e.g. `Rec."E-Mail"`). Of d17/d43/d61/cfg_walker/receiver_type, only d43/d61 read the field in production, and both seek a grammatically plain-identifier IsHandled actual (never a table field) — not bitten. d17/cfg_walker/receiver_type only reference it in test-scaffolding struct literals. No fix needed; see `.superpowers/sdd/condref-audit-report.md`
- [ ] **Number-less object identity collision (engine-wide):** `o.id.unwrap_or(0)` (`src/engine/l2/l2_workspace.rs:355/414/593`) gives EVERY Interface/ControlAddIn in an app the same internal object id `{guid}/{type}/0` — the harness-side symptom was fixed (differential keying, fix/outstanding-test-bugs), but any BY-ID lookup in L2/L3/L5/gate (FingerprintIndex, `to_location`, d64's object-id fallback) can attribute to the wrong object wherever 2+ number-less objects of one type coexist. Needs its own arc: name-qualified id component for number-less objects + golden rebaseline sweep. Population: interfaces are common in real BC apps (CDO has several)

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
