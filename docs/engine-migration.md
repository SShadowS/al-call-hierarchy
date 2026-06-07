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
| **R1a** | L2 structural body walk + ids + CFN skeleton + routine/object metadata (operations, call-sites incl. `ExpressionInfo` + result-use flags, record-ops, loops, record/scalar vars, var-assignments, condition-refs, field-accesses, `nestingDepth`, `hasBranching`, `identifierReferences`, `unreachableStatements`, normalized CFN) | **SHIPPED** — 152/152 fixtures, 0 divergences, native receiver oracles green |
| **R1b** | L2 control-context lattice + shared control-flow primitives (`controlContext` per op/callsite; IsHandled-guard elevation; TryFunction guard; error-call source-range post-pass) | **SHIPPED** — 152/152 fixtures WITH `controlContext` compared, 0 divergences, native L2-direct control-context oracle green |
| R1c (next) | L2 operation-order + scope frames (`orderId`/`frameId`/`onSuccessPath`/`dominatesSuccessReturn`; ordering oracles) | stub (below) |
| R1d | Direct capability facts (no `resourceId`) + extraction `status`/`reasons` | deferred |

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

## R1a parity status (SHIPPED — L2 structural body walk)

R1a widens parity from the identity subset to the **L2 per-routine
`IntraproceduralFeatures`** — the structural body-walk facts al-sem derives
BEFORE the call/event graph. Captured at the **post-index / pre-resolve**
boundary (`indexWorkspace`, before `resolveModel`), as an **allowlisted R1a
projection**: later-gate / L3-resolved fields (`controlContext`, `order`,
`scopeFrames`, capability facts, `tableId`/`resourceId`, the resolver-upgraded
`argumentBindings`) are STRUCTURALLY ABSENT, and the differ HARD-FAILS if any
appears on either side.

- **Corpus:** **152 / 152** source-only `ws-*` fixtures (incl. the
  `ws-r0-canon-stress` canonicalization stress fixture).
- **`cargo test --test differential` (`differential_l2_features_match_goldens`)**
  asserts the **FULL 152-fixture corpus by default** (committed default
  `R1A_L2_SET=full`; `R1A_L2_SET=small` selects the ws-d2 + ws-r0-canon-stress
  dev subset). **0 divergences, allowlist `[]`.**
- **Vectors:** `tests/l2_vectors.rs` locks **14 vector families** (two-phase
  op/callsite numbering on a mixed-kind body, `ExpressionInfo` classification,
  `argumentBindings` source kinds, result-use flags, `underAsserterror`, variable
  initializers, `unreachableStatements`, field-access vs record-op vs member-call,
  same-anchor nesting tie, non-ASCII columns, raw-text edges, repeat-loop CFN
  placement, `loopStrictlyContains` boundary, normalized CFN skeleton) in
  isolation, generated by the real al-sem L2 functions.
- **Native receiver oracles:** `tests/l2_receiver_oracles.rs` runs the
  ground-truth-free **metamorphic-receiver** + **receiver-genus-matrix** oracles
  (ported from al-sem `test/soundness/`) **natively over the Rust L2 walker** —
  NOT transitively (spec §6). They assert the **record-op vs call-site**
  classification (and `RecordOpType`) is invariant across receiver-equivalent
  syntactic variants (`with R do Op()` ≡ `R.Op()`; trigger bare `Op` ≡ `Rec.Op()`;
  temp parity) and is a record-op IFF the receiver is Record-typed (Codeunit
  facade / other-typed / compound receivers → call-site, never a fabricated DB
  op). 12 tests, all green, **no `src/engine/l2/**` fix required**. The TS oracles'
  effect-level `provenance`/`prove`/table-identity assertions depend on L3+ and are
  deferred to R1b+/R2; at L2 the oracles assert the upstream classification they
  all rest on.

### Determinism note — BYTE columns, not UTF-16 (empirical correction)

The R1a plan/spec assumed al-sem's `SourceAnchor` columns were **UTF-16
code-unit** offsets (web-tree-sitter / JS string semantics) and that the Rust
tree-sitter crate's byte columns would need UTF-16 normalization. **This is wrong
for al-sem.** al-sem parses via a **NATIVE tree-sitter parser** (the
`tree-sitter-al` native binding, not the WASM/web binding's UTF-16 `Point`
semantics), so its anchor columns are **UTF-8 byte offsets within the line** —
identical to the Rust tree-sitter crate's `start_position().column`. The committed
oracle vectors confirm this empirically: a non-ASCII line like
`Message('é'); Cust.FindSet()` yields `FindSet` `startColumn = 23` (a byte
offset) in BOTH engines; converting to UTF-16 would shift every post-non-ASCII
column down by one and BREAK parity. The Rust column path is therefore an identity
pass-through over the tree-sitter byte column (`node_util::Utf16Cols::col`, kept as
the single choke point should a future grammar/binding ever diverge).

---

## R1b parity status (SHIPPED — L2 control-context lattice)

R1b widens parity from the R1a structural body walk to the **control-context
lattice** value on every `OperationSite`/`CallSite`. It ports
`src/index/control-context.ts` (`computeControlContexts`) plus the
`routine-indexer.ts` glue, CONSUMING the validated R1a CFN skeleton as input (it
does NOT rebuild a control-flow representation). The full source-only `ws-*`
corpus differential is **152/152 green WITH `controlContext` compared**;
`KNOWN_DIVERGENCES.json` is empty.

Key facts of the port (all at parity with the TS oracle):

- **Capture point unchanged** — `controlContext` is an L2-boundary field
  (`indexWorkspace`, pre-resolve). R1a deliberately EXCLUDED it as a forbidden
  field; R1b moves it into the comparison surface. The differ still forbids
  `order`/`OperationOrder`, `scopeFrames` (R1c), capability facts (R1d), and all
  L3-resolved fields.
- **`controlContext` is ABSENT when undefined** — TryFunction / no-body / unknown
  sites carry NO field (al-sem's "assign only when defined"). The projection omits
  the key (`skip_serializing_if = "Option::is_none"`); the oracle expects ABSENCE,
  not `null`.
- **Lattice** (low → high rank, `max` accumulates the most restrictive):
  `top-level < conditional < loop-body < is-handled-guarded < error-path < unreachable`.
  Condition leaves (if/while/case headers) evaluate at the AMBIENT context, not the
  branch/loop context.
- **Shared control-flow primitives** live in `src/engine/l2/control_flow.rs`
  (`branch_termination`, `terminates`, `else_termination`, `has_explicit_else`) — a
  pure, side-effect-free port that R1c (operation-order) reuses (NOT a copy). The
  soundness invariant: `branch_termination` never returns `Fallthrough` for a
  branch that provably always terminates.
- **Error-call source-range post-pass** (`routine-indexer.ts:337-350`) — `error-call`
  ops are NOT registered in `byOperation` (the CFN leaf carries the paired
  callsite's id). The port reproduces the pairing: each `error-call` op with no
  context inherits the context of the callsite whose `sourceAnchor.{startLine,
  startColumn}` matches. (R1c needs the SAME post-pass for `order`.)
- **IsHandled-guard eligibility — exact rule** (`control-context.ts` byVarBoolParams
  + `routine-indexer.ts:302-318`): (a) by-var Boolean parameters (`p.isVar &&
  typeText == "boolean"`, case-insensitive); (b) Boolean non-parameter vars whose
  lowercased name EQUALS some whole, trimmed callsite argument text. Positive
  polarity (`if X then exit`) guards the CONTINUATION (only when exactly one branch
  falls through); negative polarity (`if not X` / `if X = false`) guards the
  then-BODY. An ineligible var (by-VALUE bool param) does NOT upgrade.
- **TryFunction guard** — a `[TryFunction]` routine yields ALL contexts undefined
  (the walker returns empty maps → every site's field is absent).

**R1b's native soundness oracle is L2-DIRECT** (`tests/l2cc_oracles.rs`), NOT a
port of the TS `reachability-crosscheck` oracle: that oracle reasons over
DOWNSTREAM L4 effect summaries, which the Rust L2 output does not have, so it would
assert nothing at L2. Instead the L2-direct oracle generates small inline AL
fixtures, drives the real walker, and asserts the lattice invariants the TS oracle
ultimately rests on, DIRECTLY on `OperationSite`/`CallSite.control_context`:
condition leaves at ambient; branch bodies ≥ `conditional`; loop bodies
`loop-body`; an `if` inside a loop stays `loop-body` (lattice `max`); single-arm
`if Bad then Error()` → error branch `error-path`, continuation unchanged; bare
`Error()`/`exit` → following sites `unreachable`; both-arms-terminating `if` →
continuation `unreachable`; `case` with a terminating arm → continuation narrows to
`conditional`, never `unreachable`; `[TryFunction]` → control-context absent on all
sites; IsHandled positive/negative polarity upgrade only the recognized region for
eligible vars (and an ineligible by-value param does not). These are independent
Rust-only invariants (not a golden diff — `tests/l2cc_vectors.rs` is the
golden-vector parity check) that catch control-context bugs the finite corpus
misses. The walker required NO fix to pass them.

**Covered vs deferred (honest):** the oracle asserts the L2 control-context lattice
invariants, NOT the full downstream effect-reachability the TS oracle adds at L4
(an `unreachable` op dropped from the routine summary, an `error-path` op not
seeding a finding on an always-raising path, `may-commit`/`commits-on-success-path`
PROVE answers). Those depend on L3 resolve + L4 summaries + digest/prove and are
R1c+/R2 surfaces. At L2 we assert the upstream lattice invariant they all rest on.

---

## R1c (next sub-gate) — STUB

R1c adds **operation-order + scope frames** to the L2 projection, reusing the
shared `control_flow.rs` primitives (R1b's branch-termination helpers) and the
same error-call source-range post-pass pattern:

- **Operation order** — `orderId` (and the `OperationOrder` envelope) on every
  op/callsite, walking the validated R1a CFN skeleton in execution order. The
  `error-call` ops need the SAME source-range post-pass for `order`
  (`routine-indexer.ts:370-381`) that R1b added for `controlContext`. The
  repeat-loop quirk (condition leaves before body) is already baked into the R1a
  skeleton.
- **Scope frames** — `frameId`/`onSuccessPath`/`dominatesSuccessReturn` per
  op/callsite (the success-path / dominator analysis over the skeleton).
- **Comparison surface** — DROP `order`/`OperationOrder` + `scopeFrames` from the
  forbidden set (still forbid capability facts (R1d) + all L3-resolved fields).
- **Native oracle** — an L2-direct **ordering-never-overclaim** oracle (like R1b's,
  NOT the downstream-effects oracle): assert the emitted order is a sound
  linearization of the CFN skeleton and that `onSuccessPath`/`dominatesSuccessReturn`
  never claim a site is on the success path / dominates the success return when the
  skeleton does not prove it.

R1c → R1d (direct capability facts) complete R1 per the spec staging. R1a's body
walk + ids + CFN skeleton and R1b's control-context are the validated substrate the
rest hang off — R1c extends, never replaces, the existing goldens.
