# Scanner validation + BCQuality detector candidates (session notes, 2026-07-16)

Working notes from a validation + research session. Purpose: pick this up later —
nothing here is committed work, no spec was written yet.

## 1. Context: the "CDO vs DO" confusion — resolved, no regression

CDO and DO are the SAME product/workspace: Continia Document Output,
`CDO_WS = U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud` (confirmed from
session history of prior cdo-gate runs).

The `alsem analyze` warning `analysis coverage degraded — 1045 unresolved
callsite(s)` is **NOT a regression** from the test-consolidation / substrate-sharing
refactors:

- It comes from the **legacy L3 advisory engine** (`src/engine/l3/coverage.rs` →
  `src/engine/gate/preflight.rs` → emitted by `src/engine/gate/run.rs`). The
  `analyze` CLI's preflight uses L3 coverage, not the fresh resolver.
- Identical count (1,045) already present on 2026-07-14, BEFORE both refactor
  merges, with `FC: no differences` byte-comparison at the time.
- The authoritative fresh resolver (`aldump --program-call-graph-stats`) on the
  same workspace: **unknown = 0 in both scopes**, output SHA-256 byte-identical
  to the recorded baseline
  `0a3b85bc832ff0a3e77acee118d203edbf62827dc37617c8d9315fe52d5cb7d0`.

Possible future task: make `alsem analyze`'s preflight consume the fresh
resolver's coverage instead of L3's, so the misleading warning goes away.

## 2. Validation results (all green, HEAD = 6b1d890)

| Check | Result |
|---|---|
| Full suite `cargo nextest run --release` | 2457/2457 pass (7 skipped = `#[ignore]`d dumps) |
| CDO gate (`ENFORCE_CDO_WS=1`) | PASS — 188/188 harness (26.2 s, shared substrate holding) + program_graph + snapshot_robustness |
| North-star on CDO/DO | unknown=0 both scopes, hash matches baseline |
| `alsem analyze` default on DO | 2,282 findings / 4,842 routines / 551 units, ~6–7 s, exit 0 |
| `alsem analyze --preset transaction-integrity` | 189 findings across 6 detectors, 25.4 s |
| `al-call-hierarchy --project <DO> --analyze` | 1,475 findings — exactly matches the documented pre-refactor baseline (366 crit / 1,109 warn), 0.3 s |

### Detector inventory + per-detector counts on DO (default run)

27 default detectors, 22 emitted findings:
d1 db-op-in-loop 828, d19 unused-parameter 601, d3 missing-setloadfields 227,
d14 dead-routine 124, d21 read-without-load 117, d11 modify-without-get 76,
d10 self-modifying-loop 72, d34 commit-in-loop 58, d9 transaction-span 33,
d8 commit-in-transaction 30, d16 obsolete-routine-call 27,
d5 set-based-opportunity 22, d12 dead-integration-event 14,
d33 unfiltered-bulk-write 12, d37 validate-without-persist 12,
d18 constant-filter-in-loop 10, d35 commit-in-event-subscriber 6,
d39 record-dirty-across-chain 4, d42 cross-call-wrong-setloadfields 3,
d45 event-transitive-table-exposure 3, d32 constant-boolean-parameter 2,
d20 unreachable-after-exit 1.

12 legitimately zero on this codebase (test-covered, healthy skip instrumentation
— e.g. d2 correctly attributes 688 opaque callees): d2, d4, d7, d13, d17, d22,
d29, d36, d38, d41, d43, d44.

7 opt-in: d40 transitive-load-missing, d46 commit-in-lifecycle, d47 io-unsafe-txn,
d48 io-in-loop, d49 uncommitted-write-before-ui, d50 checked-run-implicit-commit,
d51 retry-side-effect-duplication. Preset run emitted: d48 89, d34 58, d8 30,
d35 6, d49 5, d46 1.

Other surfaces all fine: alsem digest/prove/fingerprint/diff/events/policy,
LSP diagnostics + code lens, quality metrics.

## 3. microsoft/BCQuality — what it is

- **NOT an analyzer.** A curated knowledge base of atomic markdown files
  (YAML frontmatter) written for **LLM code-review agents** (AL-Go PR review).
  No rule IDs, no severities, no executable rules.
- **MIT licensed** — reimplementing rules in our analyzer is legally clear.
- Structure: `microsoft/knowledge/<domain>/*.md` across 16 domains
  (performance ~32 files, security ~18, style ~25+, events ~15, ui ~20+,
  privacy ~12, upgrade ~8, breaking-changes 7, data-modeling 7, telemetry 7,
  error-handling 6, testing 6, interfaces 5, appsource 4, web-services 4+,
  query 2) + a thin `community/knowledge/` layer (2 rules).
- Each rule has companion `.good.al` / `.bad.al` samples — **ideal test
  fixtures** for any detector we implement.
- Full research report (complete rule catalog, syntactic-vs-semantic
  classification, CodeCop overlap analysis) was produced in-session; key
  candidates extracted below.

## 4. Candidate new detectors (prioritized)

High value — our call-graph/dataflow moat gives an edge over anyone else:

1. **`DeleteAll`/`ModifyAll` on a `var Rec` parameter without `IsTemporary()`
   guard** (community `guard-bulk-operations-with-istemporary`). Syntactic +
   light dataflow. Silent production bulk-delete risk.
2. **Ignored `[TryFunction]` return value** — error silently swallowed.
   Syntactic: callee attribute + discarded return at call site.
3. **Event published inside a `[TryFunction]` cone** — subscriber errors
   silenced. Call-graph transitive (unique to us).
4. **Event publish inside a loop** (`do-not-publish-events-inside-loops`).
   We have d2 (fanout-in-loop); direct publish-in-loop is a distinct simpler
   check.
5. **Record cloned before `Modify`/`Delete` in a loop**
   (`avoid-cloning-records-before-modify-delete-in-loops`) — extra SQL
   round-trip per row. Needs is-clone-of-loop-var dataflow.
6. **Growing globals in `SingleInstance` subscribers** — session-lifetime
   memory leak. Structural (SingleInstance property) + unbounded-append
   detection.

Medium:

7. Query `SetFilter` after `Open()` is ignored (`set-query-filters-before-open`)
   — plus the re-open-keeps-filters variant.
8. `var Boolean` security-guard parameters on `[IntegrationEvent]`
   (`integrationevent-var-parameter-bypasses-security-guards`) — name
   heuristics (IsHandled-adjacent: HasAccess/SkipValidation/IsAllowed).
9. `repeat…Modify…until` loops in upgrade codeunits — should be
   `DataTransfer` (`datatransfer-for-bulk-init`).
10. `IsHandled` guarding critical writes
    (`do-not-bypass-critical-operations-with-ishandled`) — semantic: we know
    what code sits behind the guard.
11. `FeatureTelemetry.LogUsage` before the success path (`feature-usage-only-
    after-success`) — control-flow position.

Low:

12. HTML string-concat XSS heuristic (`al-has-no-built-in-htmlencode`).
13. API pages missing `ReadIsolation := ReadCommitted` / read-only flags
    (`expose-only-committed-data-from-api-reads`,
    `disable-write-operations-on-read-only-api-pages`).

Overlap notes: our d34/d8 already cover commit-in-loop/-transaction (BCQuality
`avoid-commit-inside-loops` adds remediation nuance only); d22 covers
flowfield-without-calcfields; d16 covers obsolete-routine-call
(`use-no-series-codeunit-not-noseriesmanagement` is a specialization).

## 5. Next steps when resuming

1. Decide first wave (suggest the six high-value ones) and write the spec
   (`docs/superpowers/specs/`), following the existing detector architecture
   (`src/engine/l5/detectors/`, registry in `detectors/mod.rs` —
   DEFAULT_DETECTORS vs OPT_IN_DETECTORS ordering matters for `detectorStats`).
2. Pull the relevant `.bad.al`/`.good.al` samples from microsoft/BCQuality as
   fixtures under `tests/fixtures/`.
3. Consider the preflight fix from §1 (fresh-resolver coverage in `alsem
   analyze`) as a standalone quick task.
