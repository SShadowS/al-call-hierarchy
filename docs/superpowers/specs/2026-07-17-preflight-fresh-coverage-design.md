# Preflight on the fresh resolver — design

**Date:** 2026-07-17
**Status:** approved (user + adversarial gpt-5.6-sol review folded in)
**Source item:** `docs/OUTSTANDING.md` → "§1 preflight fix"; origin
`docs/2026-07-16-scanner-validation-and-bcquality-candidates.md` §1.

## Problem

`alsem analyze` preflight-warns `analysis coverage degraded — 1045 unresolved
callsite(s)` on DO, a workspace where the AUTHORITATIVE fresh resolver reports
`unknown = 0`. The warning is doubly wrong:

1. It comes from the retired-advisory L3 engine, whose `unknown` definition the
   project explicitly de-authorized (CLAUDE.md "Project Direction & The Moat").
2. Worse: the gate's L3 coverage path (`L3Resolved::project_coverage_disk` →
   `project_coverage`, `src/engine/l3/coverage.rs:239`) resolves with **empty
   declared deps** and labels every app `"source"` — so the count is inflated by
   design (all dependency calls unresolvable) AND `opaqueApps` is **structurally
   empty** on this path. The current message can never even report the one fact
   it was designed for.

Additionally, the fail-closed early returns (`empty_output_result`,
`src/engine/gate/run.rs:466+`) construct a coverage with
`unresolved_callsites: vec![]` — an unreadable/fail-closed workspace preflights
**silently clean** today.

## Decision (user-approved direction)

**Always run the fresh resolver inside `analyze`** and key the preflight on its
output. Measured cost: ~3.8 s on DO (the largest real workspace; analyze goes
~6–7 s → ~10–11 s). Accepted.

## Design

### 1. Program-engine entry: `FreshCoverage`

A narrow public entry on the program-engine side (factored from what
`aldump --program-call-graph-stats` already does: `snapshot_workspace` → parse →
`program::build` → `resolve_full_program` → `Histogram`):

```rust
pub struct FreshCoverage {
    /// primaryScoped `unknown` — TRUE resolution failures (`ambiguousResolved`
    /// excluded), the realUnknownRate definition.
    pub unknown: usize,
    /// The resolve run's own coverage contract (every obligation classified).
    pub coverage_holds: bool,
    /// Files whose parse was ParseStatus::Recovered — IR may have dropped
    /// content, so unknown==0 does NOT prove completeness over them.
    pub recovered_files: usize,
    /// Symbol-only dependency apps, from the FRESH snapshot
    /// (`AppUnit::source == None`) — one engine, one dependency universe.
    pub opaque_apps: Vec<String>,
}

pub fn fresh_coverage(ws: &Path) -> Result<FreshCoverage, String>
```

NOT a bare `usize`: `coverage_holds == false` and `recovered_files > 0` can
each coexist with `unknown == 0` and must not launder into "coverage complete"
(instrument-honesty doctrine). The `Err` arm preserves the underlying snapshot/
build error text (today `resolve_full_program`'s errors get erased through
`Option`; the helper threads them out).

Out of scope (recorded as follow-ups, not silently dropped): reporting
dependency ABI-ingestion errors and declared-but-missing dependencies; sharing
one parse between the L3 and fresh passes.

### 2. Preflight semantics

`evaluate_preflight` gains a first-class **could-not-verify** state and labelled
clauses; signature moves from `(usize, &[String], bool)` to consuming
`Result<FreshCoverage, String>` + `required`:

- `Ok` with `unknown == 0 && coverage_holds && recovered_files == 0` →
  clean; opaque apps present → degraded with ONLY the opaque clause (that is a
  genuine cone limitation, kept fail-open).
- `Ok` with `unknown > 0` → degraded: `N unknown resolution edge(s)`
  ("resolution edge", not "call edge" — the histogram spans Call/Run/
  ImplicitTrigger/EventFlow).
- `Ok` with `!coverage_holds` or `recovered_files > 0` → degraded with its own
  explicit clause (`coverage contract violated`, `N recovered file(s)`); never
  the clean message.
- `Err(e)` → degraded: `coverage could not be verified: <e>`. Never
  silent-clean.
- `failed = degraded && required` (unchanged fail-open contract; the warning
  always goes to stderr, exit 4 only under `--require-dependencies`).

Clean message stays `dependency coverage complete`.

### 3. Wiring + the fail-closed hole

`gate/run.rs` calls `fresh_coverage(ws)` where it currently reads
`coverage.unresolved_callsites.len()` / `coverage.opaque_apps`. The L3
`AnalysisCoverage` struct is untouched — it still feeds
`routinesAnalyzed`/`sourceUnitsParsed` in the JSON output (verified: no
formatter/golden exposes the L3 unresolved list; formatter bytes unchanged).

The fail-closed early returns (`empty_output_result`) stop fabricating a clean
preflight: they evaluate the could-not-verify arm (reason = the fail-closed
diagnostic), so an unreadable workspace warns, and fails (exit 4) under
`--require-dependencies`.

### 4. CI-visible semantic change (CHANGELOG under Changed)

Exit-4 (`--require-dependencies`) re-keys from the inflated L3 no-deps multiset
(which included Ambiguous/MemberNotFound/ExternalTarget) to fresh `unknown` +
verify-failure. Consequences to document:

- workspaces that exited 4 can now exit 0/1 (the DO case — the point);
- fail-closed workspaces newly exit 4 under `--require-dependencies`;
- stderr wording changes (`unresolved callsite(s)` → `unknown resolution
  edge(s)` + new clauses).

### 5. Tests

- `preflight.rs` unit tests: rewrite for the new states/wording (clean /
  unknown / opaque-only / contract-violated / recovered / could-not-verify,
  each × required on/off).
- Deliberate rebaselines: the warning oracle pinning `"unresolved callsite"`
  (`tests/cli/gate_prsummary_differential.rs:636`) and the exit-codes golden
  matrix (`tests/gate-goldens/exit-codes.json` — has exit-4 cells keyed to the
  old semantics).
- New integration assertions: DO-shaped case (L3-would-warn, fresh-clean → NO
  warning) via a fixture with a symbol-only dep; verifier-error arm on the
  fail-closed path; helper count == `primary_histogram.unknown` on a fixture
  with a genuine unknown.
- `scripts/check-goldens` before commit (pre-commit hook enforces).

## Alternatives rejected

- **Lazy adjudication** (L3 first, fresh only when L3 flags): cheaper on clean
  workspaces but keeps two engines in the signal path and misses the
  L3=0/fresh>0 corner. Rejected for one-authority simplicity.
- **Message-only reword**: keeps the misleading warning. Fails the goal.
- **Bare `usize` count**: launders contract violations and recovered parses
  into "complete". Rejected on instrument-honesty grounds (review finding).

## Follow-ups (OUTSTANDING.md)

- Shared parse: L3 assembly consuming `ProgramContext::parsed()` — removes the
  duplicate parse (~halves the added cost) and the TOCTOU between the two
  passes.
- Dependency ABI-ingestion error + declared-but-missing dependency reporting in
  `FreshCoverage`.
