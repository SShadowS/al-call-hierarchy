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
- [ ] ~~Preflight shared parse~~ **DEFERRED-WITH-EVIDENCE (2026-07-17 investigation, `.superpowers/sdd/shared-parse-investigation.md`):** the duplicated work is the PRIMARY app's parse only (deps parse once, in the fresh pass) — on DO that's 407 files of a 4.8 s dep-dominated resolve, a sub-second saving; and the readers genuinely diverge on live data (DO has 4 BOM-carrying `.al` files; snapshot keeps BOM, L3 strips it — sharing without reconciling would CHANGE L3's view). Not worth the refactor + golden risk today. Wake: analyze latency becomes user-facing pain, or dep-parse caching lands (then the primary share is the remaining cost), or the BOM handling gets unified anyway
- [ ] ~~FreshCoverage ABI-error / missing-dep reporting~~ **DEFERRED — POPULATION-LESS (measured 2026-07-17):** DO has 0 ABI-ingest failures and 0 declared-but-missing deps (all 4 declared present in `.alpackages`); a real ingest failure already surfaces as `fresh_coverage` Err → could-not-verify (snapshot bails). Wake: the first real failing-ingest or missing-declared-dep population. The serde-default-empty exemption hardening rides with the same wake (its trigger is a future SymbolReference emitter shape change)
- [x] **d56 re-promotion:** added the `keyRemappedClone` skip (clone whose PK/current-key field is reassigned before the write targets a DIFFERENT row — the MoveEmailLog shape), re-promoted d56 OPT-IN → DEFAULT — DONE (`feat/d56-repromotion`); d56 emits 0 findings on DO in both the default and `bcquality` sets, both known DO false positives (MoveEmailLog current-key remap, `.dependencies` CopyLines PK remap) confirmed genuinely excluded against real source
- [ ] **Validate d61/d62/d64 on real code:** emitted 0 on DO (fixture-proven only). Run `/triage-wave` on a corpus that exercises them before considering promotion
- [x] Audit other `condition_references` consumers (e.g. d43 IsHandled-guard) for the paren/quoted-condition blind spot that bit d60; migrate to `statement_tree` where bitten — AUDITED CLEAN 2026-07-17: ground truth is narrower than described (`Parenthesized` recurses fine and always has; the real gap is only a quoted-member-field leaf, e.g. `Rec."E-Mail"`). Of d17/d43/d61/cfg_walker/receiver_type, only d43/d61 read the field in production, and both seek a grammatically plain-identifier IsHandled actual (never a table field) — not bitten. d17/cfg_walker/receiver_type only reference it in test-scaffolding struct literals. No fix needed; see `.superpowers/sdd/condref-audit-report.md`
- [ ] **Number-less object identity collision (engine-wide) — PARKED WITH WAKE (population measured 2026-07-17):** `o.id.unwrap_or(0)` (`src/engine/l2/l2_workspace.rs:355/414/593`) gives EVERY Interface/ControlAddIn in an app the same internal object id `{guid}/{type}/0`. Harness symptom fixed (differential keying). DO/CDO primary has 5 interfaces sharing one object id, BUT zero shared procedure names across them → no routine-id collapse and no observed misattribution today; the harm is latent. The fix is a golden-earthquake (name-qualified id component touches every stable-id consumer: fingerprints, baselines, digests, cache). Wake: two same-app number-less objects sharing a routine name, a misattributed production finding on an interface, or the next planned stable-id-breaking change (piggyback then)

## Deep-review remediation arcs — COMPLETE (section was stale; corrected 2026-07-17)

- [x] **T0+T1** merged (`f171d0f`); frozen CDO SHA `0a3b85bc…` re-verified 2026-07-16 ✓
- [x] **T2** crash/DoS hardening — merged `542740e`
- [x] **T3** legacy-LSP migrate-don't-patch — became the T3 LSP-migration arc (spec `2026-07-12-t3-lsp-migration-design.md`); legacy pipeline DELETED at its capstone (see CHANGELOG Removed)
- [x] **T4** hygiene — merged `d99c65e`

## Grammar (tree-sitter-al — we own it)

- [x] ~~Fix the 2 Recovered-parse grammar defects~~ **WAS ALREADY DONE — stale item (verified live 2026-07-17):** both fixed at the grammar-defects-and-repin arc (tabledata option-member collision via hidden `_tabledata_keyword` alias; `# pragma` + region/endregion `[ \t]*` tightening; see CHANGELOG "repinned to v3.1.0"), later superseded by the v3.2.0 repin. `recoveredFiles` on CDO today: **0** (re-measured)
- [ ] Work the running quirks list (low priority — engine is insulated by the lowerer, workarounds in place): spurious `left`/`operator`/`right` field pollution, `case_else_branch` inconsistency (see memory `tree-sitter-al-grammar-issues`)

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
