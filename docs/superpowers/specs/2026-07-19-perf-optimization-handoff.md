# Performance optimization — consolidated handoff (2026-07-19)

ONE-FILE summary of the entire memory/speed campaign (2026-07-17 → 07-19):
every measured fact, every verdict, the open decision, and how to continue.
A fresh session starts HERE; the detailed history lives in the referenced
docs, but nothing load-bearing is only there.

## 0. TL;DR

`alsem analyze` on Microsoft Base Application 28.0 (8,020 files, ~101k
routines) went from unrunnable to: **3-detector run 41 s** (was DNF at
90 min / 35.8 GB), **5400-file full-default ~300 s** (was 2,608 s), DO
unchanged (~9-10 s, byte-identical throughout). The full-54-detector 8020 run
still does not finish — and we now know EXACTLY why, per component:

- **Substrate (parse→L3→L4 cones+Jacobi): ~24 min / 43.5 GB — clean,
  bounded, measured.** Its levers are B1/B2 (below).
- **d1-db-op-in-loop: OUTPUT-BOUND.** It enumerates every complete evidence
  path through the dense 797-member SCC: 69.1% of its retained walk results
  are genuine complete witness paths, ~126 per walk, ~900k paths ≈ 3 h for
  its census. No behavior-preserving algorithm can beat producing its own
  output. **The next step is a SEMANTICS DECISION, not code** (§4).
- Every other detector: seconds (d19/d12 were quadratic via a shared
  fingerprint bug — fixed; everything else was never hot).

## 1. What shipped (all merged to master, all byte-stable unless noted)

| Wave | Commits | What | Effect |
|---|---|---|---|
| 1 (10 tasks) | `9c0ee77..708f000` | Demand-driven detector substrate (detectors declare requires; parity-tested), Jacobi overhaul #1 (uncertainty-edge index, serde-free change keys, take-snapshots, dirty frontier), SpanTemplates, move-not-clone hand-offs, parallel L3 parse, parallel diagnostics re-parse, FingerprintIndex/cross-ext hoists, L2→L3 moves | 8020 3-det DNF→90 s; the only output change: substrate-skipping runs omit cap-hit diagnostics (decision (a), user-approved) |
| 2a | `e2e34fc`, `136c4e2` | Structural fingerprint substitution (killed the per-finding O(F·R·L) scan — d19 988→0.23 s, d12 425→0.07 s), zero-alloc reachable + d1 touches_db memo | 5400 full-default 2608→304 s |
| 2b | `a640815`, `f9ff427` | Implicit-trigger builder parity (field-specific OnValidate + RunTrigger gate, oracle = `applicability.rs:159-227`) | Precision only — SCC-shatter hypothesis FALSIFIED (846→797); 65 edges pruned on DO, findings byte-identical |
| 2c | `511845c` | d1 walk_evidence memoized per callee (caller-independence proven, memoized≡fresh test) | Walk count O(callsites)→O(distinct callees); necessary but not sufficient |
| tracing | branch `worktree-tracing-infra` | **Permanent perf_trace layer** (§3) + the decisive attribution runs | Ends the throwaway-probe cycle; measured disabled overhead −0.93% |

Three magnitude estimates were FALSIFIED during the arc (SCC-shatter,
trigger-edge share, d1-redundancy-dominance). Standing iron law:
**measure before building** — attribution first, always.

## 2. Measured facts that still govern (clean-batch numbers)

- 8020 corpus graph: 100,941 routine nodes; 82,613 SCCs; max SCC 846→797
  (post-2b); the big SCC is SPARSE (~3 edges/member) and fused by
  direct(1067)/method(262) call cycles — trigger edges retarget INSIDE it.
- Jacobi (846-SCC): 58 passes, frontier near-full for ~20 passes then decays
  to 0; 8.34M summary-entry cardinalities across members (~9.9k/member).
  Change-test cost was 49.3 s at 5400 pre-fix, now structural. Substrate
  block at 8020 ≈ 22-27 min total.
- Cones: +11.4 GB at 8020 (per-routine transitive fact vectors — B2's
  target). Peak full-default RSS 43-52 GB, set by substrate, not d1.
- d1 census at 8020: 22,169 in-loop callsites → 7,105 walk candidates →
  4,116 distinct callee roots; ~0.3 walks/s (heavy-tailed; max single callee
  2,884 results); memo hit-rate healthy.
- **THE RATIO** (run 3, stable across 20 checkpoints): 30.9% cut results /
  69.1% complete. Cut-elision (CompleteOnly streaming) trims ≤31% — real but
  secondary.
- Historical bands for regression guards: DO default 9.0-10.7 s / ~1.6 GB;
  8020 3-det ~41-45 s / ~6.1 GB; 5400 full-default ~300 s.

## 3. The measurement infrastructure (USE THIS — no more ad-hoc probes)

`src/engine/perf_trace.rs` + ~20 permanent instrumentation points. Spec:
`docs/superpowers/specs/2026-07-18-tracing-infra.md`.

```
ALSEM_TRACE=1                  enable (Chrome trace JSON)
ALSEM_TRACE_FILE=<path>        default alsem-trace-<pid>.json
ALSEM_TRACE_DETAIL=stages|jacobi|hot   (cumulative)
ALSEM_TRACE_STDERR=1           mirror coarse spans to stderr
ALSEM_TRACE_SCC_MIN=N          per-SCC span threshold (default 100)
ALSEM_TRACE_EXIT_AFTER=<span>  early exit after a named span
```

- `hot` tier: d1 walk census + stop-kind counters + memo counters, flushed
  every 60 s AND every 1000 memo misses AND at end — cap-killed runs keep
  their data. The decisive ratio = (Σ stop-kinds − complete)/Σ stop-kinds.
- Load traces in chrome://tracing / Perfetto. File is valid mid-kill.
- Disabled path measured at −0.93% vs a no-tracing binary (noise); OFF/ON
  analysis output byte-identical (goldens + real-DO verified).
- Corpus recipe (scratchpad dies with sessions; ~1 min): see the Wave-1
  plan's Global Constraints (`docs/superpowers/plans/2026-07-18-engine-
  memory-speed-wave1.md`) — extracts Base App 28.0 source from DO's
  `.alpackages` + builds slice-5400. DO workspace:
  `U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud`.
- Measurement hygiene: quiet machine (ambient load polluted one whole batch
  — check `Get-CimInstance Win32_Processor LoadPercentage` < ~15); never two
  alsem.exe concurrently; kill stale alsem.exe before release-fast builds;
  detached runs via the scratchpad monitor-script pattern for anything
  >10 min.

## 4. THE OPEN DECISION (blocks the next d1 work — user's call)

d1 output semantics: every complete path becomes evidence
(additionalPaths + count in findings). At Base-App density that is ~900k
paths ≈ 3 h. Options:

A. **Honest cap**: keep first path + `additionalPathsCount`, cap enumerated
   extras at N with an explicit "paths capped at N" diagnostic on the
   finding (honest-caps doctrine — never silent). Expected: d1 → minutes.
   Output changes → goldens rebaseline + DO/CDO finding-level triage gate.
B. First-path-only + count (N=0 form of A).
C. Keep exhaustive enumeration; accept ~3 h d1 at this density (batch-only).

Secondary (only alongside A/B): CompleteOnly streaming in walk_evidence for
cut-ignoring consumers (≤31% trim, behavior-preserving — design sketch in
`.superpowers/sdd/w2c-design.md`-era notes and gpt's Stage A in the pi
transcript, summarized in measurements doc §9).

## 5. The remaining queue (after the §4 decision)

1. d1 semantics implementation per the decision (goldens rebaseline + triage).
2. **B1-narrow**: core-summary interning wedge (EffectIx/UncertaintyIx +
   bitset memberships INSIDE the Jacobi domain first, engine-wide B1 only
   after it proves out; preserve lexical order explicitly — never intern
   ordinals in comparisons). License with a `jacobi`-tier trace showing the
   unique-universe vs total-membership split (instrumentation already lands
   it: `largest_scc_effect_universe`).
3. **B2**: SCC-shared cone content + per-routine provenance overlays (the
   +11.4 GB cone term; members differ in subject/first-hop/tie-breaks so
   share CONTENT, not final CapabilityFact objects).
4. Deferred/rejected (reasons recorded): Salsa incrementality (cold-run
   irrelevant), corpus partitioning (can't split an SCC), parallel detectors
   (multiplies working sets at 50 GB peaks), B3 single-substrate unification
   (strategic, wrong cost — needs a detector-feature parity harness first),
   silent caps (doctrine violation), `to_lowercase` census (fold into B1's
   churn, same call sites).

## 6. Non-negotiables for ANY perf change here

- Byte-stable goldens (`scripts/check-goldens`) unless an explicit,
  user-approved output decision (like decision (a) / §4) says otherwise —
  then regen-and-TRIAGE, never blind rebaseline.
- DO byte-diff (modulo the `generatedAt` line) pre/post.
- Fresh resolver (`src/program/`) untouched by advisory-engine perf work —
  north-star SHA `0a3b85bc…` must reproduce (user runs `scripts/cdo-gate`).
- Full `cargo test` + clippy clean; rustfmt per file; explicit staging.
- Measure before building. Attribution before optimization. External review
  (pi CLI file-pointer pattern, gpt-5.6-sol has full context of this arc)
  for any design-level change; source-verify every external claim.

## 7. Where everything lives

- This handoff: the entry point.
- `docs/superpowers/specs/2026-07-17-engine-memory-speed-findings.md` — the
  original root-cause review (§1-§8) + Wave-1 outcome (§7b).
- `docs/superpowers/specs/2026-07-18-wave2-measurements.md` — SCC anatomy,
  Jacobi telemetry, detector profiles, Waves 2a/2b/2c outcomes, §9 the
  decisive d1 attribution.
- `docs/superpowers/specs/2026-07-18-tracing-infra.md` — the tracing spec.
- Plans: `docs/superpowers/plans/2026-07-18-engine-memory-speed-wave{1,2a,2b,2c}.md`,
  `2026-07-18-perf-trace-infra.md`.
- `docs/OUTSTANDING.md` — the live queue (kept current).
- Session-scratch artifacts (verdict files, traces) are ephemeral; every
  load-bearing number is IN this file or the specs above.
