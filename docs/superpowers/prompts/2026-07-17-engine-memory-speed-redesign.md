# Super-prompt: engine memory + speed design review (multi-model)

Paste everything below this line into a fresh Claude Code session in
`U:\Git\al-call-hierarchy`.

---

## Mission

Evaluate the alsem engine's CURRENT design for memory and speed, and propose better
designs — up to and including deep refactors — so that `alsem analyze` runs FASTER
and consumes LESS memory while keeping the SAME or MORE features and future
possibilities (more detectors, more semantic layers, bigger corpora). Two solution
tracks are explicitly on the table:

- **Track A — compat-preserving:** internal refactors; every golden/output surface
  byte-stable.
- **Track B — compat-breaking:** output shapes, ids, goldens, stored baselines may
  all change if the design win justifies it. We control every downstream consumer
  (CLAUDE.md "Testing Philosophy"); breaking is allowed when it buys real
  architecture.

This is a DESIGN review with measurements — not an optimization PR. Deliverables are
evidence + a ranked design proposal (spec-grade for the top candidate), not code.

## The motivating observation (measured 2026-07-17)

`alsem analyze` on Microsoft Base Application 28.0 source (8,020 plain `.al` files,
ONE app, zero dependencies, only 3 cheap detectors selected):

- never finished inside 10 minutes (killed twice);
- one run was later found alive at **7.1 GB RSS**;
- 2,674-file slices of the SAME corpus finish in ~3 min each → time superlinear,
  memory strongly superlinear in corpus size;
- for scale: DO (Continia DocumentOutput, 551 units) analyzes in ~7-11 s.

The user's driving question: **how does so little source code consume so much
memory?** 8k files of AL is maybe 40-80 MB of text; the engine turns it into 7+ GB.
Find out exactly where, then judge whether the design that causes it is the right
one.

## Reproducible pathological corpus (1 minute to build)

```bash
python -c "
import zipfile, re, json, os
ws = r'<SCRATCHPAD>/baseapp-ws'   # use the session scratchpad dir
os.makedirs(ws, exist_ok=True)
z = zipfile.ZipFile(r'U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud/.alpackages/Microsoft_Base Application_28.0.46665.48632.app')
mani = [n for n in z.namelist() if n.endswith('NavxManifest.xml')][0]
xml = z.read(mani).decode('utf-8', 'ignore')
g = lambda a: re.search(f'{a}=\"([^\"]+)\"', xml).group(1)
json.dump({'id': g('Id'), 'name': g('Name'), 'publisher': g('Publisher'), 'version': g('Version'), 'dependencies': []}, open(ws + '/app.json', 'w'))
[z.extract(n, ws) for n in z.namelist() if n.endswith('.al') and n.startswith('src/')]
"
```

Slice it (basename-copy subsets of `src/`) for scaling curves. System Application
(`Microsoft_System Application_...app`, 1,309 files, ~11 s) is the mid-size point.
Run with: `target/release-fast/alsem.exe analyze <ws> --detector
d61-ishandled-bypasses-critical-write,d62-telemetry-before-success,d64-api-page-write-surface
--format json` (cheap detectors → isolates SUBSTRATE cost). Build with
`cargo build --profile release-fast --bin alsem` (never full `--release` for
iteration — CLAUDE.md). Kill stale `alsem.exe` before rebuilding (holds the lock).

## Method (in order — measurements license designs)

### Phase 1 — Profile (no opinions before numbers)

1. Scaling curve: 1k / 2k / 2.7k / 5.4k / 8k slices; wall time + peak RSS each
   (`peak_rss.py` exists in `scripts/`). Identify the superlinear term empirically.
2. Phase attribution: time + RSS-delta per engine stage. Instrument or bisect:
   parse → L2 feature projection → L3 assembly (`assemble_and_resolve_workspace*`)
   → event graph → L4 (SCC/effect summaries/capability cones) → L5 detector loop →
   the NEW fresh-preflight resolve (`fresh_coverage` in
   `src/program/resolve/full.rs`, prepended to every analyze since the
   preflight-fresh-coverage arc) → formatting. A `--verbose`/env-gated timing print
   or `Instant` probes on a scratch branch is fine (throwaway, don't commit).
3. Memory attribution: what is RESIDENT at peak? Heap-profile or estimate
   structurally: count × size of the big Vecs (see hypotheses). On Windows,
   `peak_rss.py` + staged early-exits (build only through stage N, print RSS) is a
   workable poor-man's profiler.

### Phase 2 — Structural hypotheses to verify or falsify (from prior code reads;
none are confirmed — treat as suspects, cite file:line evidence for each verdict)

- **String-id explosion:** internal ids are long heap Strings
  (`"{appGuid}/{type}/{num}"` object ids, routine ids with embedded 64-hex hashes)
  cloned into EVERY node, op, callsite, edge, finding, map key. 8k files ×
  hundreds of sites × ~100-byte Strings compounds fast. The LSP surface interns
  (`string-interner` is already a dependency!) — the L2/L3/L5 engine does NOT.
- **Per-detector FingerprintIndex rebuild:** every `detect_dNN` calls
  `FingerprintIndex::build(&ws.routines, &ws.objects)` — 40+ detectors × full
  rebuild over all routines. O(detectors × workspace).
- **L2 `PFeatures` weight:** per-routine Vecs (call_sites with argument_texts +
  argument_infos + argument_bindings, statement_tree clones, identifier_references,
  condition_references, anchors everywhere) — much of it duplicated INTO L3
  (`L3Routine` clones statement_tree, condition_references, etc. — grep the
  `.clone()`s in `l3_workspace.rs` assembly).
- **Event graph pair costs:** publishers × subscribers shapes (Base App has
  thousands of events).
- **L4 capability cones / witness digests:** the witness-perf arc (see memory +
  git history) fixed 80 min → 14 min on CDO but explicitly DEFERRED a
  "flow-insensitive §7" step — that residual has never been measured on a corpus
  this dense.
- **Double substrate:** analyze now materializes BOTH the fresh program engine
  (snapshot + graph + parsed IR, dropped after preflight) AND the L2/L3 model —
  sequentially, but each alone may be multi-GB at 8k files.

### Phase 3 — Design evaluation (after numbers)

For each hot spot: is it accidental (fixable in-place, Track A) or ARCHITECTURAL
(the data model itself is wrong for scale, Track B)? Questions worth asking:

- Should the engine intern ALL symbols/ids (u32 keys + one arena) like the LSP
  surface already does? What does that do to golden/output shapes (ids appear in
  every golden — Track B)?
- Should L3 BORROW from L2 (or L2 be projected lazily per-routine, streamed into
  detectors) instead of cloning eagerly into resident whole-workspace Vecs?
- Should detectors run over a streaming per-object view instead of a fully
  materialized workspace (bounded-memory analyze)?
- Is one shared substrate for fresh-engine + L3 (single parse, single symbol
  table) now justified? (A prior shared-parse investigation deferred it on cost
  grounds — `.superpowers/sdd/shared-parse-investigation.md` — but Track B changes
  the calculus.)
- What future features does each design unlock or foreclose (incremental analyze,
  multi-app workspaces, larger-than-RAM corpora, parallel detector execution)?

### Phase 4 — Multi-model review (REQUIRED)

Get independent takes from BOTH external models via the pi MCP (`mcp__pi__pi_ask`),
then reconcile:

- Models: `gemini-3.1-pro-preview` and `gpt-5.6-sol`. Default thinking (high).
- **KNOWN BUG + workaround (do not skip):** pi_ask silently DROPS long prompt
  bodies (the delegate replies "no proposal present"). Write your brief to a FILE
  first (profiling results + hypotheses verdicts + design options + repo paths) and
  send a SHORT prompt: "Read the brief at <absolute path> and answer its
  questions." Proven pattern.
- Give them: the measured phase/RSS attribution, your Phase-2 verdicts with
  file:line cites, the Track A/B option sketches, and the key files
  (`src/engine/l2/features.rs`, `src/engine/l2/l2_workspace.rs`,
  `src/engine/l3/l3_workspace.rs`, `src/engine/l4/`, `src/engine/l5/detectors/mod.rs`,
  `src/engine/l5/fingerprint.rs`, `src/program/resolve/full.rs`,
  `src/snapshot/`). Ask each for: holes in the attribution, designs we missed,
  which track they'd pick and why, and what they'd measure next.
- **Treat every external claim as unverified** (the evidence contract is routinely
  ignored) — source-verify each load-bearing claim against the code before
  adopting it. This has caught real value AND real errors in past reviews.

### Phase 5 — Deliverables

1. `docs/superpowers/specs/<date>-engine-memory-speed-findings.md` — the evidence:
   scaling curves, phase attribution table, hypothesis verdicts (file:line),
   external-model inputs with adopted/rejected dispositions.
2. A ranked design proposal: Track A quick wins (with expected numbers) and the
   Track B architecture (data-model sketch, what breaks, migration/golden story,
   what future features it unlocks). Top candidate at spec grade, ready for
   `superpowers:writing-plans`.
3. Update `docs/OUTSTANDING.md` (add the chosen items; this doc's findings replace
   the "profile analyze scaling" idea).
4. Commit docs. NO engine code changes in this session beyond throwaway
   instrumentation on a scratch branch.

## Hard constraints + house rules

- Read `CLAUDE.md` first and follow it (build commands, golden discipline,
  rustfmt-per-file, `scripts/check-goldens`, never `git add -A`).
- The north-star behavior anchor: `aldump --program-call-graph-stats` on
  `U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud` currently hashes SHA-256
  `0a3b85bc832ff0a3e77acee118d203edbf62827dc37617c8d9315fe52d5cb7d0`. ANY future
  implementation of these designs must reproduce resolution SEMANTICS (Track B may
  change output SHAPE, never resolution TRUTH — same edges, same taxonomy counts).
- Doctrine: measure the population before building; fix root causes, never
  symptoms; the best solution, not the quickest (time is not a constraint).
- Detector capability floor: everything `registered_detectors()` does today must
  remain expressible in any proposed design (the substrate reference in CLAUDE.md's
  BCQuality-wave section lists what detectors actually consume).
- Windows quirks: don't pipe long runs through `| tail` (masks exit codes);
  background Bash calls die at 10 min — split long runs into slices or checkpoints;
  CR/CRLF checks use `file`/`od`, never grep.
