# Change-Impact Wedge — brainstorm input

Pre-brainstorm working document (2026-07-18, written at the v1.0.0 release point).
This is NOT a spec — it frames the product conversation the charter calls for. The
brainstorm session decides; this doc just makes sure it starts from the engine's
real capabilities and the honest open questions.

> **STALENESS WARNING (read first if picking this up after a refactor).** The
> "What already exists to build on" table and the file:line references below are a
> point-in-time snapshot of the engine at commit `b7da82d` (master, 2026-07-18). A
> major refactor between then and the brainstorm can move or rename any of them —
> re-verify each cited path/symbol against current code before treating it as fact,
> and re-answer the Q1/Q2 architecture forks against the refactored shape (a refactor
> that ports effects onto the fresh graph may already have decided the biggest fork).
> The PRODUCT framing (why-now, cone semantics, tiering, the eight forks) is
> refactor-independent and stays valid regardless.

## Why this, why now

The semantic-intelligence charter names the **change-impact wedge** as the product
feature the moat exists for: given a change (a routine, a field, a table, an event),
answer *what is affected* — precisely, with evidence, across apps. Everything the
last arcs built converges here:

- **Zero-real-unknown whole-program graph** — impact cones are only trustworthy when
  edges are; ours are (0 unknown on the reference workspace, honest taxonomy for the
  rest, instrument-hardened after the deep review).
- **Event-flow edges** — data-is-control-flow (charter): publisher→subscriber and
  cross-extension fan-out are already first-class edges, so impact crosses the event
  boundary instead of stopping at it (the thing no text-search tool can do).
- **Per-route evidence** — every edge carries its dispatch shape + evidence class
  (source/catalog/ABI/conditional/dynamic), which maps directly onto impact
  CONFIDENCE tiers instead of a flat "maybe affected" soup.
- **L4 effect summaries** (SCC condensation, per-routine effects) — the substrate
  for "affected HOW" (reads/writes table X, commits, UI), not just "affected".
- **v1.0.0 shipped** — the engine surface is stable enough to build a consumer on.

## What "impact" must mean here (charter framing)

Reverse reachability over the resolved graph, seeded by a change description:

```
seed (routine / field / table / event / object)
   → reverse cone over call + event + implicit-trigger edges
   → per-path evidence + conditions carried along
   → ranked, tiered answer: PROVEN affected / conditionally / dynamically-possible
```

The differentiator is honesty at the edges: a cone that includes conditional
(interface-dispatch-under-precondition) and dynamic reaches as LABELED TIERS, never
silently merged with proven paths — the same taxonomy discipline as the resolver.

## Candidate product shapes (for the brainstorm to rank)

1. **`alsem impact <selector>` (CLI, likely first).** Selector names a routine/
   object/field; output = the tiered reverse cone with per-path witness chains
   (JSON/terminal/SARIF-adjacent). Cheapest to ship: pure consumer of the existing
   report; the fingerprint/witness machinery (cli-b) already renders call chains.
2. **Diff-mode impact (`alsem impact --diff <base>`)** — seed from what CHANGED
   (git diff → changed routines/fields via the def-surface fingerprints that
   already exist for the LSP's rung gating). This is the CI story: PR annotation
   "this change reaches posting routines in 2 dependent apps", test-selection
   hints, breaking-change gating with `--fail-on-tier`.
3. **LSP surface** — "Show impact" code lens / custom request reusing the cone
   engine live in the editor (the multi-root work means cross-app impact inside
   one editor session).
4. **BC-Brain feed** — impact cones as MCP-served knowledge for agent consumers
   (ties into the separate bc-brain product track; probably a v2 consumer, not
   the wedge itself).

## What already exists to build on (verified, with owners)

| Capability | Where | State |
|---|---|---|
| Resolved whole-program edges + taxonomy | `src/program/resolve/` (`ClassifiedEdge`, `Histogram`) | v1.0, zero-unknown |
| Reverse adjacency | `ctx.graph.edges_by_from` (forward); reverse index would be new but trivial | forward-only today |
| Event edges + cross-extension subscribers | resolver `EventFlow` edges; L5 event graph | first-class |
| Witness chains rendering | cli-b fingerprint/query machinery | ships today |
| Effect summaries (what a routine DOES) | `src/engine/l4/` (advisory L3-based) | exists; L3-basis caveat below |
| Changed-surface detection | `src/lsp/def_surface.rs` fingerprints | exists (LSP-scoped) |
| Selector grammar | cli-b fingerprint selectors | exists, reusable |

## Open design questions (the brainstorm's agenda)

1. **Seed granularity.** Routine-level first (edges exist natively)? Field/table
   seeds need the L4/effect layer or new field-use indexing on the FRESH engine —
   the current effect summaries live on the ADVISORY L3 side; the wedge deciding to
   consume them makes L3 load-bearing again (counter-doctrine) vs porting effects
   onto the fresh graph (new work, cleaner). This is the biggest architecture fork.
2. **Cone semantics.** Pure reverse-call reachability, or effect-aware ("affected"
   = can OBSERVE the change: reads what the seed writes)? The former is shippable
   now; the latter is the real promise and needs effects-on-fresh.
3. **Tiering + cycles.** How do conditional/dynamic edges compose along a path
   (weakest-link tiering?), and how are SCC cycles presented (collapse per L4's
   condensation?)?
4. **Diff seeding.** Reuse def-surface fingerprints (body vs surface change
   distinction is already computed) or a simpler git-diff→routine mapping? What
   does a SIGNATURE change seed vs a body change seed?
5. **Output contract.** New envelope kind vs extending the analyze report; SARIF
   fit (it's not findings — is a "finding per affected site" framing honest or
   noise?); determinism/golden strategy from day one.
6. **Depth/size controls.** Real cones on BC apps will be huge (posting routines
   reach everything). Default depth? Tier-filtered? Per-hop cost display? The
   L5 witness perf lessons (path explosion, capping discipline) apply directly.
7. **Cross-app scope.** Workspace-primary seeds only, or seed inside a dependency
   (e.g. "Microsoft changes X in the next BC version — what in MY app breaks")?
   The latter is the killer ISV story and the graph already spans deps.
8. **Name.** `impact` / `blast` / `cone` / `reach` — pick once, it's the brand.

## Suggested first-wave scope (strawman for the brainstorm to attack)

`alsem impact <routine-selector>`: routine seeds, reverse call+event cone,
weakest-link tiering (proven/conditional/dynamic), depth + tier filters, JSON +
terminal output with witness chains, deterministic + golden-tested, workspace and
whole-program scopes mirroring the stats command. Explicitly OUT: field seeds,
effect-awareness, diff mode, LSP surface — each a fast-follow once the cone core
is proven.

## Process expectations

Full pipeline: brainstorm (user session — the forks above are product decisions) →
spec in `docs/superpowers/specs/` (external adversarial review round, as with the
preflight arc) → plan → SDD execution. The engine-side prerequisite worth deciding
early because it gates everything: **reverse index + effects-on-fresh (Q1/Q2)**.
