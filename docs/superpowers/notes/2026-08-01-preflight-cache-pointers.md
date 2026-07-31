# Preflight caching — design pointers (Fable)

Everything below is verified against master `c0396d0` source, file:line cited. Read §1
first — two attributions in the brief are wrong in ways that change the design.

---

## 1. Corrections to the brief (read these first)

**1a. The empty `AbiCache` is NOT why dependency work repeats across runs — and it is
not the lever the census implies.** `AbiCache` (`src/program/abi_ingest.rs:126-224`) is a
process-level, in-memory `Mutex<HashMap>` caching parsed `SymbolReference.json` for
SYMBOL-ONLY deps. Two facts kill the framing:

- It dies with the process regardless. `build_context_res` constructing it fresh
  (`src/program/resolve/full.rs:1107`) costs nothing *across runs* — a process cache
  can never survive a run. It matters only if `build_dep_layer` runs more than once in
  one process (LSP updater rebuilds — a separate concern; `alsem analyze` calls it once).
- On DO the deps ship **embedded source**, so they are parsed as source, not ingested as
  ABI. The ABI-ingest path `AbiCache` covers is a slice of `dep_layer`'s 141.9 ms, not
  the 1,139 ms parse. Persisting the ABI cache is a ~tens-of-ms lever on DO and a 0 ms
  lever on 8020.

**1b. Cross-run dependency caching partially EXISTS already — extraction, not parsing.**
`src/snapshot/cache.rs` is a content-addressed on-disk cache (`cached_source`, key =
blake3 of the whole `.app`, `~/.cache/al-ch-snapshot-cache/<hash>.json`) for the
zip-extraction of embedded dep source. So on every warm run, dep *text* loads from a JSON
cache; what repeats from scratch is the tree-sitter parse + IR lowering of ~11,000 dep
files (`parse_snapshot`, `src/snapshot/parse.rs:85`) and node extraction
(`build_dep_layer`, `src/program/build.rs:70`). This file is also the house pattern to
copy for atomic writes and fail-open reads (see §6).

**1c. Your `compute_gate_model_instance_id` suspicion is CONFIRMED — it is path-only and
fatal as a cache key.** `src/engine/gate/model_instance_id.rs:82-88`: the parts are
`"{guid}@{version}"` plus `ws:<rel_posix>` strings — file *names*, never file *content*.
Edit any file body → same id → a cache keyed on it serves a stale verdict. It also does a
second `discover_al_files` disk walk (admitted at `src/engine/gate/run.rs:210-213`).
Do not reuse it, do not imitate it, and do not add a third walk (§4).

**1d. One framing correction on value: the whole-verdict cache does NOT help the edit
loop at all.** Its key must cover primary source content (§4), so any workspace edit is a
guaranteed miss and a full recompute. It is an *identical-input rerun* optimization: CI
re-runs, running `alsem` twice for two `--format`s, re-running after a no-op. The lever
that helps the *edit* loop (dep content unchanged, primary edited — the actual dev
workflow) is the dependency-artifact cache, which is a much bigger design (§2, §5).

Everything else in the brief checks out: DO ~95 % dep parse arithmetic is consistent with
`parse.rs` (one `al_syntax::parse` per file, all source-bearing apps, deps included), and
the four-scalar-throwaway description matches `fresh_coverage`
(`src/program/resolve/full.rs:1263-1293`).

---

## 2. Which lever first, and the honest ceiling of each

**Build Lever 1 (whole-`FreshCoverage` verdict cache) first.** It is small, provably
sound (§3), and its ceiling on the measured pain point is the whole thing:

| Lever | DO ceiling | 8020 ceiling | When it pays | Cost/risk |
|---|---|---|---|---|
| **1. Verdict cache** (cache `FreshCoverage`, keyed on content) | ~2.1–2.2 s of 2.64 s per warm hit (snapshot_build ~459 ms + key hash remain as the warm floor; parse/dep_layer/assemble/resolve/ctx_drop all skipped) | ~3.06 s of 3.50 s | Identical-content reruns only (1d) | Tiny artifact (~1 KB); the entire risk is key completeness |
| **2. Dep-universe artifact cache** (persist dep-derived parse/extract products) | up to ~1.1 s parse share + ~142 ms dep_layer, on EVERY run incl. after primary edits | ~0 (no deps) | The edit loop — the common case | Real design work; IR not serializable today; two soundness holes to close first (§5, §9) |
| **3. Persist `AbiCache`** keyed by `.app` content hash | tens of ms | 0 | always | trivial, but must NOT reuse its current version-based key (§8) |
| 4. Narrow the preflight resolve to primary scope | part of resolve_full (536 ms DO / 1,838 ms 8020) | part of 1.8 s | always | NOT a cache — a verdict-semantics change; currently unsound to do blindly (§9-Q1) |

Lever 1's warm floor is `snapshot_build`, deliberately: the sound key derives *from the
snapshot* (§4), so you keep paying ~459 ms (DO) to build it. Trying to beat that floor
with a cheaper pre-snapshot key means a second discovery implementation that can drift
from the real one — the exact defect class 1c documents. Accept the floor.

Recommendation: spec Lever 1 + Lever 3 now (Lever 3 is a two-line key change away from
sound once a content hash is in reach); treat Lever 2 as a follow-up spec with §9's
must-verify list as its Task 0. Do not let Lever 2's complexity delay Lever 1.

---

## 3. Is the whole-verdict cache a trap under "no silent clean"? No — with three rules

`fresh_coverage(workspace_root)` is a pure function of (bytes on disk under the workspace
+ discovered `.app` bytes, engine binary). It reads nothing else — no config, no
thresholds, no env that changes the verdict (checked: `AnalyzeArgs` config/baseline/
severity flags all act downstream of `fresh`, `run.rs:206-209`). Memoizing a pure
function on a *content-complete* key is not "reporting a verification that didn't
happen" — it is replaying a verification that DID happen, on inputs proven byte-identical.
The contract survives if and only if:

**Rule 1 — the key is content-complete (§4).** Every stale-verdict scenario is a key
gap. There is no payload-level defense: a lying-but-well-formed entry under a correct key
is undetectable without recomputing, so ALL soundness lives in the key + entry integrity.

**Rule 2 — every abnormal state means recompute.** Missing entry, unreadable entry,
JSON/schema mismatch, self-hash mismatch, version-tuple mismatch → miss, silently (log at
debug), never an error. `snapshot/cache.rs:44-55` already models this fall-through.

**Rule 3 — never cache the `Err` state.** `fresh` is `Result<FreshCoverage, String>` and
the `Err` ("could-not-verify") state is first-class in `evaluate_preflight`
(`src/engine/gate/preflight.rs:55-63`). An `Err` is not a pure-function output — it
captures transient environment (locked file, dying disk). Caching it would launder a
one-time I/O flake into a *persistent* could-not-verify (or worse, the inverse: caching a
verdict and replaying it when the workspace has since become unreadable — impossible if
the key derives from a freshly built snapshot, which is another reason for §4's design).
Cache `Ok` only — including degraded `Ok`s (`unknown > 0` etc.); they are as
deterministic as clean ones and the key changes on any edit anyway.

**One consequence people will miss: the cached payload is byte-gated OUTPUT, not just an
exit code.** `run.rs:398-400` copies `fresh.opaque_apps` into the coverage block the
Json/Terminal/Html formatters render. So the entry must round-trip `FreshCoverage`
exactly — `opaque_apps` stored verbatim in its produced order (already deterministically
sorted at production, `full.rs:1248-1250`; do not re-sort on load, store-as-is proves
itself under the cold-vs-warm byte test in §7). This also means the byte-identical merge
gate gets a new required form: cold run and warm run must produce identical bytes.

---

## 4. Cache key design

**Principle: derive the key FROM the one snapshot the preflight already builds. No
second walk, no independent hasher that can drift from discovery.** Concretely: split
`build_context_res` (or inline in `fresh_coverage`) so Step 1 (`SnapshotBuilder.build()`)
runs first, compute the key from the resulting `AppSetSnapshot`, look up, and only on
miss continue into parse → dep_layer → assemble → resolve.

### What MUST be in the key

Fold, length-prefixed (the house discipline — `sig_fp::write_len_prefixed`,
`sha256_of_strings`), over:

1. **Engine identity: hash of the running binary itself** (`current_exe()`, blake3,
   memoized in a `OnceLock`; ~50 MB at >1 GB/s ≈ tens of ms, once). This subsumes in one
   stroke: resolver behaviour version, builtin catalogs, grammar version (tree-sitter-al
   is *compiled in*), lowerer changes, schema of the cached struct's semantics. Do NOT
   use a hand-bumped constant as the primary defense — the repo has a live proof that
   discipline rots: `CACHE_VERSION_GRAMMAR` still says `"tree-sitter-al-v2.5.2-native"`
   (`src/engine/gate/cache_prune.rs:40`) while the actual grammar is v3.2.0 — it was
   never bumped through two grammar upgrades, and nothing failed. A binary hash cannot
   rot. Cost: a rebuild with identical code invalidates the cache (embedded timestamps)
   — that fails toward recompute, which is the correct direction. Keep ONE hand-bumped
   `PREFLIGHT_CACHE_SCHEMA` const *additionally*, for the entry's serialization shape
   only (§6).
2. **Workspace app identity + world**: `snap.workspace_app` (guid, name, publisher,
   version), `snap.world`.
3. **Per app in `snap.apps`, in a canonical order (sort by guid, then version, then
   content hash)**: guid, name, publisher, version, tier, `declared_deps` (verdict-
   load-bearing: `opaque_dependency_closure` BFSes over them, `full.rs:1215-1247`, and
   resolution visibility is closure-scoped), `internals_visible_to` (friend map feeds
   resolution, `build.rs:127`), `compilation` context (currently near-inert for
   lowering, but cheap and future-proof against the deferred preproc-fidelity item),
   and the app's **source-content identity**:
   - EmbeddedSource dep → the `.app` blake3 that `cached_source` already returns
     (`snapshot/cache.rs:38-40`) — free, already computed, already content-addressed.
   - SymbolOnly dep → the same `.app` blake3 (`app_content_hash`,
     `src/snapshot/embedded.rs:50`); verify it is actually retained on the `AppUnit` for
     this tier, compute if not.
   - Workspace/local-repo source → a fold of `(virtual_path, len, text)` per file. **Do
     NOT use the existing `SourceRoot.content_hash` for this** — see the trap below.
4. **The full discovered app set, not the reachable closure.** `load_all_apps` loads
   every `.app` in ancestor `.alpackages` without app.json filtering (documented at
   `full.rs:1181-1184`), and those unrelated packages become graph nodes; event-subscriber
   wiring is whole-snapshot-scoped (test comment, `abi_ingest.rs:1133-1136`), so an
   unrelated package can in principle influence the verdict. Folding all of `snap.apps`
   covers this automatically; an added/removed unrelated package causes a spurious miss —
   acceptable, fails toward recompute.

`.dependencies/` law: keying from `snap.apps` satisfies it by construction — the key
never sees folder names, only discovered app content. Any design that enumerates
directories by name to build the key is both a law violation and a drift risk.

### Where path-vs-content hashing bites (three confirmed instances)

- `compute_gate_model_instance_id`: path-only (1c). Fatal. Don't touch it for this.
- **`SourceRoot.content_hash` for walked source is content-INCOMPLETE**: `walk_al_source`
  hashes `f.text.as_bytes()` only, concatenated — no virtual_path, no length prefix
  (`src/snapshot/provider.rs:59-64`). Two collisions it permits: (a) renaming a file
  (same texts, different names — mostly harmless to the verdict but `recovered_files`
  paths and witness identity disagree), and (b) **re-splitting bytes across file
  boundaries** — one file split into two whose concatenated bytes match hashes
  identically while parsing completely differently. That one is verdict-changing. Either
  fix `walk_al_source`'s fold (check consumers: `verify.rs` identity checks read this
  field) or — safer — compute the cache key's own per-file fold and leave
  `content_hash`'s definition alone.
- `AbiCache`'s key is `(guid, name, publisher, version)` (`abi_ingest.rs:121-124`) —
  version-keyed, not content-keyed. Fine in-process; **unsound the moment it persists**:
  rebuilding a dep `.app` at the same version with different content is routine in dev.
  Persist under `app_content_hash` instead (Lever 3).

### The cheapest key that is still sound

`blake3( binary_hash ‖ canonical_fold(snapshot) )`, 64 hex chars, used as the cache
filename (the `<64-hex>.json` convention `cache_prune.rs:151-156` already validates).
Marginal cost over a run that already builds the snapshot: the workspace per-file fold
(~5–10 MB on DO, few ms) + string folds. The `.app` hashes are already paid for by
`cached_source`. Total warm overhead ≈ snapshot_build + <20 ms.

---

## 5. What is actually worth caching (ranking)

Ranked by (recompute saved) ÷ (bytes × deserialization + design risk):

1. **`FreshCoverage` verdict** — ~1 KB, microseconds to load, saves 2.2 s (DO warm).
   Unbeatable ratio. Build first.
2. **Parsed `SymbolReferenceAbi` per symbol-only dep** (Lever 3) — small JSON already in
   hand at ingest; keyed by `.app` blake3. Cheap, always-on win, tens of ms.
3. **Dep-universe artifact** (Lever 2) — the edit-loop lever. What it must contain to
   remove dep *parse* from a miss-path run, under CURRENT verdict semantics:
   dep-extracted `ObjectNode`/`RoutineNode` sets (the `DepLayer` contribution),
   per-dep `recovered_files` paths, AND everything the resolver derives from dep
   *bodies* — because the resolve is whole-program ("all source-bearing routines in all
   apps", `ProgramReport` doc `full.rs:182-183`), `coverage_holds` spans all edges, and
   `recovered_files` spans all units. That means either (a) serialize dep-internal
   classified edges + obligations (≈25k edges on CDO: wholeProgram 43,375 − primary
   18,113), or (b) change the verdict semantics to primary-scoped (§9-Q1 — a
   contract change, not an optimization). Two hard traps before any of this is
   speccable — see §9-Q2/Q3.
4. **Raw parsed IR on disk — recommend AGAINST.** Three reasons: `al_syntax::ir` has
   zero serde derives today (grepped — none; only `SourceFile` in snapshot/embedded.rs
   is serializable), so this is a whole-crate serialization surface + schema-versioning
   burden; parse is *cheap per byte* (0.098 ms/file — 1.1 s wall, parallel, for ~11.6k
   files) while serialized IR for ~114 MB of dep text would be hundreds of MB whose
   deserialization + allocation plausibly costs the same order as re-parsing; and it's
   dominated by option 3, which persists the much smaller *derived* products instead.
   This is exactly the "cache that costs more to load than to recompute" failure named
   in the brief.
5. **Resolved graph / full model — no.** Largest artifact, most schema churn,
   and `RoutineNodeId` embeds `AppRef(u32)` — a per-run interning index
   (`build.rs:76-77`), meaningless across processes. Any persisted node/edge artifact
   must store app identity symbolically (guid) and re-intern on load; forgetting this
   produces silently-wrong graphs, not errors. This trap applies to option 3 as well —
   design its serialization around symbolic app identity from day one.

Also name the non-cache alternative once in the spec so it's a considered rejection, not
a blind spot: a resident daemon / watch mode keeps the model in memory and sidesteps
serialization entirely — the LSP server already IS that shape (`lsp::updater` reuses
`Arc<DepLayer>` across rebuilds). Plausibly the eventual right answer for the edit loop;
out of scope for a CLI-invocation cache, but the spec should say why.

---

## 6. Invalidation, corruption, concurrency, location

- **Entry format**: self-describing JSON — `schemaVersion`, the full key (for audit; the
  filename is the lookup), a versions tuple (binary hash + `PREFLIGHT_CACHE_SCHEMA`),
  the `FreshCoverage` payload, and a self-integrity hash over the entry
  (`cache_prune.rs`'s `artifactContentHash` literal-replacement recompute,
  `cache_prune.rs:139-144`, is the working precedent). Tmp+rename makes torn writes
  near-impossible; the self-hash is cheap belt-and-suspenders for bit-rot and hand
  edits, and it's what makes the §7 poisoned-entry tests expressible.
- **Concurrency**: copy `persist_cache` (`snapshot/cache.rs:82-89`) — per-process tmp
  name, rename over destination, last-wins; concurrent writers of the same key write
  identical bytes by construction (same key ⇒ same inputs ⇒ same verdict). Two engine
  versions sharing the dir never collide — the binary hash is in the key. Windows: rename
  can fail on a sharing violation if a reader holds the destination — warn + continue
  uncached (the existing pattern already does).
- **Schema evolution**: `PREFLIGHT_CACHE_SCHEMA` bump ⇒ old entries fail the tuple check
  ⇒ miss ⇒ recompute; prune deletes them. Never migrate in place.
- **Location**: the repo now has THREE cache concepts — `~/.al-sem/cache` (dep artifacts,
  byte-pinned al-sem-parity prune report), `<os-cache>/al-ch-snapshot-cache` (extracted
  source, unversioned, unbounded, no pruning), and this new one. Do NOT put the verdict
  cache in `~/.al-sem/cache` — its artifact schema and prune stdout are golden-pinned to
  al-sem wording; a foreign file shape lands in `removed-unreadable`. Recommend a new
  versioned root (`<os-cache>/alsem/preflight-v1/`), a `--no-cache` (or env) escape
  hatch, and a size/age-bounded prune. Flag as an adjacent cleanup: `al-ch-snapshot-cache`
  has no pruning at all and grows forever on a disk-constrained machine (the U:-drive
  memory) — same prune pass could cover it.
- **Test hermeticity**: the cache dir MUST be overridable (env var or injected path).
  Note the existing snapshot cache is NOT — every test that touches `.app` extraction
  shares global mutable state. Don't repeat that; the §7 tests are impossible to
  hand-state without an isolated dir.

---

## 7. Testability under this repo's doctrine

The organizing trick that satisfies "hand-state the precondition" and "prove
discrimination" in one move: **use a deliberately-WRONG payload as a tracer for which
path ran.** Write a well-formed cache entry whose payload says `unknown: 999` — a value
the real workspace can never produce — into a temp cache dir, by literal `Write`, never
by asking production code to mint it. Then every test is a two-outcome oracle: result
carries 999 ⇒ the cache was served; result carries the true value ⇒ recompute ran. Both
directions are observable, so every test below has a built-in can-fail proof.

All tests call the PRODUCTION entry point (`run_analyze_with_exit` or the new
`fresh_coverage` wrapper) — never the internal lookup helper (test-pins-function-not-use
is this repo's five-times-caught defect).

1. **Hit works at all**: hand-write entry with tracer payload under the key computed for
   fixture workspace W → run → assert 999 surfaced (via `PreflightResult.unknown_edges` /
   the degraded stderr message). Proves the cache is actually consulted by the USE.
2. **Key completeness table** (the soundness core — one test per key component):
   construct workspace pairs in tempdirs differing in exactly one input; assert the
   computed keys differ. Rows: flip one byte in a primary `.al` body; add a file; rename
   a file; **re-split one file into two with identical concatenated bytes** (pins the
   length-prefix/path fold — this row FAILS against today's `SourceRoot.content_hash`,
   which is its discrimination proof pre-recorded); delete a dep `.app`; replace a dep
   `.app` with same (name, version) but different bytes (pins content-over-version);
   edit `app.json` `dependencies`; edit `internalsVisibleTo`; vary the engine-identity
   component. Discrimination proof per row: remove that component from the fold, watch
   the row fail, restore. For scripted breaks, assert the patch applied
   (`assert s.count(old) == 1`) — the 2026-07-31 lesson.
3. **Fail-closed corpus**: for each of {truncated JSON, wrong `schemaVersion`, version-
   tuple mismatch, self-hash mismatch, entry file deleted}, seed a tracer entry, apply
   the corruption, run → assert the TRUE verdict (not 999) is returned AND (where
   applicable) a fresh valid entry replaced the bad one. The tracer makes "recompute
   happened" directly observable without instrumenting production code.
4. **Stale-content never served**: seed tracer entry for W, then flip one byte in W →
   run → true verdict, not 999. (This is test 2 exercised through the USE.)
5. **Never-cache-Err**: make the snapshot fail (e.g. id-less root `app.json` — the
   documented fail-closed layout), run with an empty cache dir → assert could-not-verify
   surfaced AND the cache dir is still empty. Then fix the workspace, run → clean, and a
   fresh entry appears. Discrimination: temporarily cache the `Err` and watch the second
   half fail.
6. **Cold-vs-warm byte identity** (the merge-gate form): run `run_analyze_with_exit`
   twice on a fixture with a real opaque dep (so `opaque_apps` is non-empty and flows
   into the JSON coverage block), empty cache then warm cache → assert stdout bytes,
   exit code, and stderr warning identical. This pins the §3 output-bearing consequence.
7. **CDO ratchet interaction**: the existing CDO-gated zero-ratchets must run with the
   cache DISABLED or a cold dir — spec this explicitly so the north-star measurement
   never accidentally measures a warm replay of itself. (`scripts/cdo-gate` should export
   the no-cache env.)

---

## 8. Codebase traps (consolidated)

- `model_instance_id` is path-only (1c) — never a content key. `run.rs:210-213`.
- `SourceRoot.content_hash` (workspace tier) is a boundary-free concat of texts —
  rename- and re-split-collisions. `provider.rs:59-64`. Don't reuse as-is; if you fix it
  instead, audit `snapshot/verify.rs` + `snapshot.rs:154-265` consumers first.
- `AbiCache` key is version-based, not content-based — in-memory OK, persistence
  unsound. `abi_ingest.rs:121-124`.
- `AppRef` is per-run interning — any persisted node/edge artifact must use symbolic app
  identity. `build.rs:76-77`.
- Hand-bumped version constants rot silently — `CACHE_VERSION_GRAMMAR` is two grammar
  major versions stale (`cache_prune.rs:40`). Prefer binary-hash identity.
- `opaque_apps` is formatter-visible output, not just exit-code input — `run.rs:398-400`.
- `~/.al-sem/cache`'s artifact shape + prune stdout are al-sem-golden-pinned — don't
  co-locate new artifacts there.
- The snapshot source cache has no dir override and no pruning — pre-existing debt the
  new cache must not copy.
- `evaluate_preflight` warns on stderr on EVERY degraded run independent of
  `--require-dependencies` (`run.rs:467-477`) — the warm path must reproduce the warning
  identically (comes free if `FreshCoverage` round-trips exactly).

## 9. Open questions the follow-up spec (Lever 2 / scope-narrowing) must answer first

- **Q1 — verdict-semantics scope.** `FreshCoverage.unknown` is primary-scoped but
  `coverage_holds` and `recovered_files` are whole-program (`full.rs:182-211,
  1273-1277`). Any "skip dep bodies" design changes what the gate vouches for. If the
  team decides the preflight should vouch primary-scoped only, that is a deliberate,
  documented contract change with its own review — never a silent side effect of caching.
- **Q2 — is dep-edge resolution provably independent of primary content?** Event-
  subscriber wiring is whole-snapshot-scoped (`abi_ingest.rs:1133-1136`). If a dep
  subscriber could ever bind a primary publisher (impossible in compiled BC, but the
  engine resolves by name at snapshot scope), then dep-derived edges are NOT a pure
  function of the dep universe, and a dep-universe artifact keyed only on dep content is
  unsound. Must be settled by reading `resolve::event` wiring + a hand-built fixture,
  before Lever 2 is specced.
- **Q3 — obligation/edge serializability.** Dep-internal `ClassifiedEdge`s carry
  witnesses/spans/`RoutineNodeId`s; decide the symbolic-identity serialization (Q2's
  sibling) and measure the artifact size on DO before committing (measure the
  population before building taxonomy for it — house doctrine).
