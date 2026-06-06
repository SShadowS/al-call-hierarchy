# Engine migration: al-sem → Rust (differential, TS-oracle)

The al-call-hierarchy engine is being re-derived in Rust under `src/engine/` by
**differential migration against a TypeScript oracle** — the sibling `al-sem`
analyzer (`U:\Git\al-sem`). al-sem stays the source of truth; the Rust engine is
grown layer-by-layer and proven byte-equivalent to al-sem at each gate before the
next layer is attempted. Nothing in `master`/the shipping LSP binary is touched —
all migration work lives on the `engine` branch under `src/engine/`, `tests/`,
and `docs/`.

## Gates

| Gate | Scope | Status |
| --- | --- | --- |
| **R0** | Source identity parity (objects + routines: stable ids, signature fingerprints, canonical signature text) | **SHIPPED** — full source-only corpus differential green |
| R1 (next) | L2 index parity: capability extractors, `IntraproceduralFeatures`, control-context | stub (below) |

---

## R0 parity contract

R0 proves the Rust extractor derives the SAME **identity subset** as al-sem for
every source-only workspace fixture. The identity subset is, per object:

```
{ stableObjectId, name, kind (objectType), signatureFingerprint }
```

and per routine:

```
{ stableRoutineId, name, kind (RoutineKind),
  signatureFingerprint, normalizedSignatureHash, canonicalSignatureText }
```

Key contract points:

- **modelInstanceId is pinned to `"r0"`** when al-sem dumps the goldens, but the
  identity subset is **independent of it** — the stable ids derive from
  `appGuid/objectType/objectNumber` (objects) and `stableObjectId#normalizedSignatureHash`
  (routines), never from the per-run `modelInstanceId`. The pin is
  belt-and-suspenders + honors the plan contract; it makes the internal RoutineId
  host/path-independent without affecting what R0 compares.
- **signatureFingerprint is return-type-aware.** al-sem's `contracts.ts` was fixed
  (the earlier controller-reviewed fix) to re-derive the routine signature from the
  real `r.returnType`. Post-fix, for EVERY routine:
  `signatureFingerprint == normalizedSignatureHash == sha256(canonicalSignatureText)`
  `== the "#"-suffix of stableRoutineId`. The harness compares BOTH the
  `stableRoutineId` AND the `signatureFingerprint`, and the goldens carry the
  pre-hash `canonicalSignatureText` so a signature drift is human-readable (a
  SHA-256 mismatch alone gives no locality).
- **The suffix invariant** (`stableRoutineId` ends with `#<normalizedSignatureHash>`)
  is implicit in how both engines construct the id and is exercised by every
  routine row in the corpus.
- **Identity encoders are byte-exact ports.** `src/engine/ids.rs` reproduces
  al-sem's hashing — notably `sha256OfStrings` length-prefixes each part with its
  **UTF-16 code-unit count** (JS `String.length`), not byte/scalar length. Locked
  independently of the goldens by `tests/encoder_vectors.rs` against
  `tests/r0-vectors/encoder-vectors.json` (48 committed vectors).

### What the extractor reproduces (and the oracle quirks it must honor)

`src/engine/snapshot.rs` (`snapshot_workspace(dir) -> IdentitySnapshot`) is the
structural walker. It deliberately mirrors al-sem field-for-field:

- objects: `src/index/object-indexer.ts` (`OBJECT_TYPE_MAP`, `extractObjectNumber`,
  `extractObjectName`, recursive object-decl discovery). `permissionsetextension`
  is intentionally absent — al-sem skips it, so we must too.
- routines: `src/index/routine-indexer.ts` (`classifyAndCollectAttributes`,
  `getReturnTypeText`, `collectDescendants` prune-at-match).
- params: `src/index/intraprocedural-refs.ts` (`extractParameters`).
- strip: `src/parser/ast.ts` (`stripQuotes`).
- attribute name: `src/index/attribute-from-node.ts`.

Two oracle quirks the full-corpus differential surfaced (and the Rust side now
reproduces exactly — see `KNOWN_DIVERGENCES.json` policy below; the allowlist is
empty because both were FIXED, not allowlisted):

1. **Quoted-name parameters are dropped.** al-sem's `extractParameters` finds the
   parameter name via the first `identifier` child; a parameter whose name is a
   *quoted* identifier (`"Sales Header": Record "Sales Header"`) has only a
   `quoted_identifier` name node and is **skipped entirely** from the canonical
   signature. Driven by `ws-r0-canon-stress` (`DoWork`).
2. **appGuid fallback to `"unknown"`.** al-sem reads the appGuid ONLY from
   `<root>/app.json` (no subdir search); on any failure — missing file,
   unparseable JSON, or missing/non-string `id` — it defaults to the literal
   `"unknown"` and never throws. Multi-app fixtures that keep their app.json under
   subdirs (`a/app.json`, `b/app.json`) therefore resolve to `unknown:...` for
   every object. Driven by `ws-diff-coverage-narrowed`.

Also note: `[InternalEvent]` classifies as `procedure` (NOT event-publisher) — only
`eventsubscriber` / `integrationevent` / `businessevent` change a routine's kind.
This matches al-sem (and deliberately differs from the LSP parser, which treats
InternalEvent as a publisher).

---

## How to run

Default `cargo test` is fully **OFFLINE** — no Bun, no al-sem checkout, no env
vars. Everything the differential needs is committed under `tests/r0-corpus/`
(source fixtures), `tests/r0-goldens/` (goldens + `manifest.json`), and
`tests/r0-vectors/` (encoder vectors).

```bash
# offline differential (R0 exit gate) + encoder vectors + everything else:
cargo test

# just the differential harness:
cargo test --test differential

# LIVE REFRESH (regenerate + re-copy goldens/fixtures from al-sem) — NOT in the
# default loop; requires Bun + the al-sem checkout:
AL_SEM_DIR=/u/Git/al-sem cargo test --test differential -- \
    --ignored refresh_goldens_from_al_sem --nocapture
```

The refresh test (a) shells `bun run scripts/dump-goldens.ts` in `$AL_SEM_DIR`,
(b) copies every source-only `ws-*` fixture + its `*.golden.json` + `manifest.json`
into `tests/r0-corpus/` and `tests/r0-goldens/`, (c) prints al-sem git sha +
grammar sha + engine sha for provenance, and (d) does NOT auto-commit — it leaves
a reviewable diff. If `AL_SEM_DIR` is unset it skips (a stray `--ignored` run is a
no-op, not a failure).

### Golden-refresh procedure (when al-sem identity logic changes)

1. In al-sem: make the change, `bun test` green, `bun run scripts/dump-goldens.ts`
   regenerates all goldens deterministically. Commit + push al-sem.
2. In the engine worktree: run the live-refresh command above to re-copy.
3. `cargo test --test differential` — fix any new divergence in `src/engine/`
   (extractor) until green with an empty allowlist, then commit the engine.

---

## Grammar provenance & convergence

al-sem's committed goldens were produced with the **`tree-sitter-al` `v2.5.2-shim`**
grammar:

- al-sem `GRAMMAR_VERSION = "tree-sitter-al-v2.5.2-native"`, package version `2.5.2`.
- `v2.5.2-shim` == commit **`89b1d055214d95bcf9596e168b240df313bd1a36`** on
  `github.com/SShadowS/tree-sitter-al`. That commit carries the committed
  `src/parser.c` (~24 MB) + `src/scanner.c`.

The engine's bundled submodule `tree-sitter-al/` uses the SAME remote but was
pinned to the stale **`v2.0.0`** commit `a9dc044ea07e773d974c9f772b1a8cae7001d5ab`.
Convergence (R0 Task 6, done LAST so the harness catches any AST-shape delta) was
simply **advancing the submodule gitlink pin** to `89b1d05`:

| revision | grammar | role |
| --- | --- | --- |
| `a9dc044` | `v2.0.0` | stale engine-submodule pin (pre-convergence) |
| `89b1d05` | `v2.5.2-shim` | oracle grammar (al-sem goldens) — convergence target |

`build.rs` already compiles `tree-sitter-al/src/parser.c` from the submodule
(default `TREE_SITTER_AL_PATH="tree-sitter-al"`), so the swap needed no `build.rs`
change. The bundled `tree-sitter-al/` was never a separate fork — it is the
canonical remote at a stale pin — so `.gitmodules` is left unchanged.

**Consequence:** the engine now parses with the EXACT grammar that produced the
goldens, so any differential divergence is a real extractor bug, not a
grammar-version artifact.

---

## `KNOWN_DIVERGENCES.json` policy

`KNOWN_DIVERGENCES.json` (repo root) is an array of
`{ fixture, path, reason, expires }`. The differential test FAILS if:

- (a) any divergence is NOT covered by an exact `(fixture, path)` entry
  (undocumented divergence), OR
- (b) any allowlist entry is UNUSED this run (over-broad/stale entries are not
  allowed).

Matching is EXACT on the `(fixture, path)` pair — never prefix/glob.

**Divergences are FIXED in the Rust extractor or DOCUMENTED here — never silently
normalized and never hacked into the golden.** Entries must be narrow, justified,
and expiring. If a divergence reveals an al-sem oracle bug (Rust is "more
correct"), that is a controller/review decision — STOP and flag it; do not change
al-sem's identity logic unilaterally (cf. the earlier signatureFingerprint fix).

**An empty allowlist is the goal — and is the R0 exit state.**

---

## Current parity status

- Corpus: **157 / 157** source-only `ws-*` fixtures (156 al-sem fixtures + the
  `ws-r0-canon-stress` identity stress fixture).
- `cargo test --test differential`: **157 fixtures, 0 divergences, allowlist
  empty (`[]`).**
- `KNOWN_DIVERGENCES.json` = `[]`.

R0 (source identity parity) is **shipped**.

---

## R1 (next gate) — STUB

R1 widens parity from the identity subset to the **L2 SemanticIndex**: the
structural facts al-sem derives per routine BEFORE the call/event graph.

Target surface (to be pinned in an R1 plan + golden extension):

- **Capability extractors** — `src/index/capability/*` facts (table writes,
  commits, event publishes, background/dispatch classification, file/blob/http
  capability rows) with their confidence + provenance.
- **`IntraproceduralFeatures`** — per-routine structural flags (`hasBranching`,
  loop presence, identifier references, etc.) that al-sem indexes for summaries.
- **Control-context** — per-operation order/dominance metadata
  (`controlContext`, `onSuccessPath`, `dominatesSuccessReturn`, frame ids) used
  later by the transaction-span and commit-ordering analyses.

R1 keeps the same machinery: a return-type-aware oracle dump from al-sem, an
offline golden corpus, the live-refresh path, and the empty-allowlist exit
criterion. The identity subset proven in R0 becomes the join key the L2 facts
hang off, so R1 extends — not replaces — the R0 goldens.
