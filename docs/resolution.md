# Call resolution: what gets resolved, and how it's measured

Every feature in this project — the call hierarchy, the analyzer, the impact-analysis
queries — rests on one thing: for each call in your code, does the engine know what it
calls? This document is the honest accounting of that.

## The buckets

`aldump --program-call-graph-stats <workspace>` classifies every call it finds. The JSON
keys are the ones the tool emits.

| Bucket | JSON key | Meaning |
|--------|----------|---------|
| Resolved (source) | `resolvedSource` | Target found in your workspace or a dependency that ships source |
| Resolved (catalog) | `resolvedCatalog` | A platform built-in (`Record.Modify`, `Page.RunModal`, …). These are compiler intrinsics — they appear in no `.app` symbol file, so the engine carries a hand-built catalog of them |
| Resolved (dependency symbols) | `resolvedAbiExternal` | Target found in a dependency's `SymbolReference.json` when it ships no source |
| Conditionally resolved | `conditionalResolved` | Resolved under a stated precondition — interface dispatch, for example, where the target depends on which implementation is passed in |
| Provably dynamic | `honestDynamic` | The target genuinely isn't decidable before runtime. There is no static answer to find |
| Provably empty | `honestEmpty` | There is no callee at all — an event with no subscriber, for instance |
| **Unresolved** | `unknown` | **A real failure: the engine should have found a target and didn't.** This is the number that matters |
| Ambiguous, resolved | `ambiguousResolved` | A closed set of same-object overloads, all of them known. Not a hole, so not counted as unresolved |

The distinction that matters for using the tool: `honestDynamic` and `honestEmpty` are
*answers*. `unknown` is a *failure*. A tool that folds the two together can look precise
while being blind.

## Current measurement

Measured on CDO — Continia's real Business Central workspace — immediately after the
Tier-1 deep-review-remediation merge (commit `f171d0f`). JSON SHA-256
`0a3b85bc832ff0a3e77acee118d203edbf62827dc37617c8d9315fe52d5cb7d0`.

Two scopes are reported. `primaryScoped` counts calls in the workspace's own code —
this is the one that describes what you'd see analyzing your project. `wholeProgram`
additionally counts calls *inside* dependencies.

| Scope | total | resolvedSource | resolvedCatalog | resolvedAbiExternal | honestDynamic | honestEmpty | conditionalResolved | unknown | ambiguousResolved |
|-------|------:|----------------:|-----------------:|----------------------:|----------------:|-------------:|----------------------:|--------:|--------------------:|
| primaryScoped | 18113 | 8325 | 5783 | 57 | 55 | 3876 | 17 | **0** | 0 |
| wholeProgram | 43375 | 10219 | 5783 | 57 | 55 | 26942 | 319 | **0** | 0 |

`realUnknownRate` is **0.0000%** in both scopes.

Read that as a point-in-time measurement to re-verify, not a permanent property. The
review that produced it also found that the metric had previously been *structurally
unfalsifiable* — a missed call could be miscounted as a built-in, vanish entirely, or
never be measured at all. Those holes were closed first; the zero above is the first
measurement taken with the fixed instrument.

## Reproducing it

```bash
aldump --program-call-graph-stats path/to/workspace
```

The numbers in the table need CDO, which only exists on machines with access to it. The
full gated suite — the zero-unresolved ratchet, the coverage contract, the exact
`ambiguousResolved` pin — runs as:

```bash
scripts/cdo-gate <path-to-cdo-workspace>
```

That script sets `ENFORCE_CDO_WS=1`, which turns "workspace missing, skip the test" into
a loud failure. Without it, those tests skip silently when the workspace isn't there —
which is why CI can't run them and a person schedules the gate locally.

## Two things that are easy to confuse

**Which engine produced a number.** `aldump --program-call-graph-stats` is the current
resolver and the authoritative one. `aldump --l3-call-graph-stats` and its siblings are
an older engine kept for advisory comparison; they report under a different key
(`legacyL3UnknownRate`) and count some cases differently, so the two numbers are not
directly comparable even when both are non-zero.

**Which definition of "unresolved."** `realUnknownRate` excludes `ambiguousResolved`,
because a closed set of known overloads is not a hole. The earlier definition counted
those as unresolved, and is still reported alongside as
`realUnknownRateLegacyIncludingAmbiguous` — so a change in how the metric is *defined*
can never quietly look like a change in how well the engine *works*.

## Preflight coverage

`alsem analyze` checks resolution coverage before it reports anything. If dependencies
are missing, the engine sees less of your program and therefore finds fewer problems —
which reads exactly like a clean codebase. By default that prints a warning; with
`--require-dependencies` it exits 4 instead, so a build can't pass for the wrong reason.
