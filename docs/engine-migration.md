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
| **R1c** | L2 operation-order + scope frames (`orderId`/`frameId`/`onSuccessPath`/`dominatesSuccessReturn` per op/callsite; routine `scopeFrames[]`; error-call order post-pass) | **SHIPPED** — 152/152 fixtures WITH `order`+`scopeFrames` compared, 0 divergences, native L2-direct structural ordering oracle green |
| **R1d** | L2 direct capability facts (the 13 `extractCapabilities` family extractors run on PRE-RESOLVE routines: `op`/`resourceKind`/`confidence`/`provenance`=direct/`via`=self/`resourceArgSource`/`extra`/witness ids; STRIPPED of L3 `resourceId` + nested `table-field.tableId`; `op:"publish"` excluded — L4-injected) + extraction `status`/`reasons` + unreachable-filtered index diagnostics | **SHIPPED** — 152/152 fixtures WITH capability facts compared, 0 divergences, native L2-direct capability oracle green |
| **R1 (= R1a+R1b+R1c+R1d)** | Full L2 per-routine feature parity at the `indexWorkspace` (pre-resolve) boundary | **COMPLETE** — all four sub-gates SHIPPED; 152/152 corpus differential green across the entire L2 surface + a native L2-direct oracle per sub-gate |
| **R2a** | L3 record-type resolution (source-only): resolved `recordVariable.tableId` / `recordOperation.tableId` (declared vars → ops → implicit `Rec`/`xRec` in Table/Page/Extension triggers) + merged TableExtension fields, captured POST-RESOLVE / PRE-SUMMARY in StableTableId form | **SHIPPED** — 153/153 fixtures with resolved tableIds + merged extension fields compared, 0 divergences, the anti-degenerate coverage matrix healthy (rv=228, op=281, implicitRec=76, extFields=9), native L3-direct record-type oracle green |

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

## R1c parity status (SHIPPED — L2 operation-order + scope frames)

R1c widens parity from the R1b control-context lattice to the **operation-order
index** — `OperationOrder` (`orderId`/`frameId`/`onSuccessPath`/
`dominatesSuccessReturn`) on every op/callsite, plus the routine's
`ScopeFrame[]` table — at parity over the source-only `ws-*` corpus. The walk
(`src/engine/l2/operation_order.rs`, a faithful port of al-sem
`src/index/operation-order.ts:computeOperationOrder`) is PURE over the validated
R1a CFN skeleton + the lowercased `attributesParsed` names, REUSING R1b's shared
`control_flow.rs` branch-termination primitives (NOT re-derived). The **full L2
differential is 152/152 green WITH `order`+`scopeFrames` compared**, 0
divergences, `KNOWN_DIVERGENCES` empty.

The walker is correct as-is — the native structural ordering oracle
(`tests/l2order_oracles.rs`) and the byte-parity differential BOTH pass with **no
`src/engine/l2/**` change required** for the exit gate.

Notes that fell out of the port (worth remembering for R1d / R2):

- **Capture point = POST-INDEX / PRE-RESOLVE** (unchanged). `order` + `scopeFrames`
  are computed during `indexRoutines` so `indexWorkspace` already carries them;
  R1a/R1b EXCLUDED them as forbidden, R1c moves them into the comparison surface.
  The differ still forbids capability facts (R1d) + all L3-resolved fields
  (`tableId`/`resourceId`/resolver-upgraded `argumentBindings`).
- **`order` absent-when-undefined.** Assigned only when the walk produced an entry
  (`if ord !== undefined`); ABSENT (key omitted, never `null`) for symbol-only /
  no-body / TryFunction routines. `scopeFrames` is omitted when empty.
- **`scopeFrames` present-when-a-root-exists.** A present-but-empty body tree still
  emits the root frame (kind `block`, parentFrameId -1) even with zero orders — so
  scopeFrames is NOT omitted just because there are no operations. TryFunction →
  empty (no orders, NO scopeFrames). `statementTree` undefined → no root frame.
- **Branch-frame `false`-field serialization trap.** A branch frame (if-then /
  if-else / case-branch) ALWAYS emits `branchAlwaysTerminates` AND
  `branchMayFallThrough` (even when `false`), and emits `branchTerminatesBy` only
  when it always-terminates (`exit`/`error`). Root/loop/try frames OMIT all three.
  Modeled as `Option<bool>`/`Option<String>` with `skip_serializing_if =
  "Option::is_none"` (NOT `is_false`, which would wrongly drop the `false`).
- **Per-leaf `orderId` gaps + aliases are VALID.** `orderId` increments once per
  leaf assignment (BEFORE checking which ids are present). A leaf with BOTH an
  op-id and a callsite-id clones the SAME `OperationOrder` into both maps;
  `exit`/`error` leaves consume an orderId even when not projected. So emitted
  orderIds may have GAPS and ALIASES — the oracle asserts RELATIVE pre-order
  (`<`), never global density/uniqueness.
- **Error-call order post-pass** runs in the EMITTER layer (`l2_workspace.rs` /
  `apply_operation_order`), NOT in the pure walk — the CFN skeleton dropped
  anchors, but the op/callsite RECORDS carry `source_anchor`. For each
  `error-call` op with no order, find the callsite whose
  `source_anchor.{startLine,startColumn}` matches and COPY its full
  `OperationOrder` verbatim (no new orderId, no inferred frame). Identical to
  R1b's controlContext post-pass.
- **`onSuccessPath` exit-vs-error.** Uses the EXACT check `term != "error"` (NOT
  `!terminates`): **exit-arms ARE on the success path** (exit is a normal return);
  **error-arms are NOT**. Unreachable-after-bare-exit/error sites → false. A bare
  top-level `Error()` follows the AMBIENT `onSuccessPath` (usually `true`) — error
  leaves are NOT force-set to false.
- **`dominatesSuccessReturn` timing + the loop-contained-exit caveat.** True ONLY
  for a reachable op DIRECTLY in the routine ROOT block with no prior
  normal-return-possible statement. `normalReturnPossibleBeforeHere` is updated
  AFTER visiting each if/case (the construct's own leaves never see its own
  update; only LATER root statements do). The `"other"`/default wrapper PROPAGATES
  the caller's value; block/if/case/case-branch/loop/try + all conditionLeaves +
  error/exit leaves reset it to false. **Loops are NOT treated as
  normal-return-possible** — an `exit` inside a loop does NOT set the flag, so
  `dominatesSuccessReturn` is NOT a full postdominance proof (the oracle
  intentionally does not assert the loop-contained-exit case; R1d/R2 must treat
  the flag as a sound-but-incomplete dominator signal).

**R1c's native soundness oracle is L2-DIRECT** (`tests/l2order_oracles.rs`), NOT a
port of the TS happens-before oracle. The TS `ordering-never-overclaim*` /
`ordering-metamorphic*` tests reason over the DOWNSTREAM `src/digest/ordering.ts`
happens-before graph (`buildHBEdges`/`dom`/`mayCoExecute`: a `must_all_paths` edge
only when `dom`, no edge between exclusive sibling branches, no intra-iteration
loop edge). That graph is an R2 (digest) surface the L2 output never builds — so we
assert at the L2 boundary the STRUCTURAL ordering facts the HB graph rests on:
non-root-frame ops never claim `dominatesSuccessReturn`; error-arm ops are never
`onSuccessPath` while exit-arm ops are; the frame chain is well-formed (every
frameId resolves, parent chains terminate at the root, branch flags match
termination); and relative pre-order holds (condition before owning op, if-cond
before then before else, case selector before branches, loop condition before body
incl. the repeat quirk). The downstream never-overclaim edges are DEFERRED to R2.

---

## R1d parity status (SHIPPED — L2 direct capability facts) — R1 COMPLETE

R1d widens parity from the R1c operation-order surface to the **direct capability
facts** — the output of al-sem's 13 `extractCapabilities` family extractors run
INTRAPROCEDURALLY on each routine — reusing the now-validated R1a body walk + R1b
control-context + R1c operation-order substrate. With R1d SHIPPED, **R1
(= R1a + R1b + R1c + R1d) is COMPLETE**: the full L2 per-routine feature surface is
at parity, 152/152 corpus fixtures green WITH capability facts compared, 0
divergences (`KNOWN_DIVERGENCES.json` empty), plus a native L2-direct oracle per
sub-gate.

Notes that fell out of the port (the capture-point discipline is the subtle part):

- **Capture point = `extractCapabilities` on PRE-RESOLVE routines, NOT the L4
  summary.** The extractors are L2-LOGICAL (their `ExtractionContext` is just
  `routine.features` + a variable index + declared-type `receiverTypeOf` + the
  unreachable filter — no call graph, no L3 resolution, no summaries), but in
  production they RUN during the L4 summary pass. R1d invokes them at the L2
  boundary (`project_named_routine` → `extract_capabilities`), exactly as
  al-sem's pre-resolve dump does. The Rust orchestrator
  (`src/engine/l2/capability/mod.rs`) mirrors `extractor.ts` + the
  `summary-runner.ts:509-513` opaque override.
- **`publish` is L4-INJECTED — EXCLUDED at L2.** `summary-runner.ts` mints one
  `op:"publish"` fact per published event from the RESOLVED `model.eventGraph`;
  `extractEvents` at L2 emits SUBSCRIBE only. So a publisher routine
  (`[IntegrationEvent]`) emits ZERO direct facts at L2; a subscriber
  (`[EventSubscriber]`) emits exactly one `subscribe` fact. The oracle asserts
  both, and that `op:"publish"` never appears anywhere at L2.
- **status/reasons = the EXTRACTOR's own output, NOT L4-augmented coverage.**
  `RoutineSummary.coverage` adds uncertainty-derived reasons (from
  `summary.uncertainties`) and downgrades complete→partial — that augmentation is
  L4. R1d compares the extractor's RETURN value, replicating ONLY the L2
  opaque override (`!bodyAvailable` → `["opaque-dependency"]`; `parseIncomplete` →
  `["parse-incomplete"]`; both clear facts + force status `unknown`).
- **L3 identity STRIPPED — structurally, not as a post-pass.** `resourceId`
  (TableId/EventId/ObjectId) and the nested `table-field.tableId` on every
  `ValueSource` (reachable via `resourceArgSource`, `extra.bodyArgSource`/
  `keyArgSource`/`valueArgSource`, and recursively through `constant-var.initializer`)
  are NOT DECLARED on the Rust serde projection types — so they CANNOT serialize.
  The differ + the native oracle's recursive key scan both hard-fail if either
  appears. Resource-id-bearing facts (table) therefore stay `confidence:"unresolved"`
  at L2 (they only become `"static"` once the id resolves in R2).
- **Unreachable filter depends on R1b controlContext.** Capability extraction drops
  any op/callsite whose `controlContext == "unreachable"` (effects never come from
  unreachable code — the soundness core) and emits an index-stage `info` diagnostic
  for each excluded site. The diagnostics are part of the compared surface. The
  receiver-genus classification (record-op vs member-call on a Codeunit) reused from
  R1a keys the table-vs-not distinction.

**R1d's native soundness oracle is L2-DIRECT** (`tests/l2cap_oracles.rs`), NOT a
golden diff (that is `l2cap_vectors.rs`, byte-parity with al-sem) and NOT the L4
effect/digest/prove oracles. It asserts the L2 direct-capability CONTRACT directly on
each emitted `CapabilityFact`: every fact is `provenance=="direct"` + `via=="self"`;
NO fact carries `resourceId`/`tableId` anywhere (recursive scan over the serialized
JSON, incl. nested `ValueSource`s); NO `op:"publish"` exists (publisher → zero facts,
subscriber → one `subscribe`); an `unreachable` op produces NO fact + emits an index
diagnostic (the same record op yields a table fact iff reachable); a table fact's
witness op is a record op on a Record-typed receiver (a member call on a
Codeunit-typed receiver yields none); and the SOFTENED confidence rule
(`confidence=="static" ⇒ resourceId present`), asserted concretely as "a table fact
is `unresolved`, never `static`, at L2". As of this gate every case passes with no
`src/engine/l2/capability/**` change required — the extractors were correct.

### Covered vs deferred (R1d / R2)

- **Covered (L2 direct surface):** the direct capability facts + extraction
  status/reasons + unreachable-filtered diagnostics, STRIPPED of L3 identity.
- **Deferred to R2/L4:** inherited / cone capability facts + their provenance
  (`provenance != "direct"`, composed in L4 summaries); the L4-injected
  `op:"publish"` facts (from the resolved eventGraph); the L3-resolved
  `resourceId`/`tableId` + the resolved `confidence` upgrade; the L4-augmented
  coverage status/reasons.

---

## R1 retrospective (R1a + R1b + R1c + R1d — full L2 per-routine parity)

With R1d shipped, the **entire L2 per-routine feature surface** is byte-equivalent to
al-sem at the `indexWorkspace` (pre-resolve) boundary, 152/152 corpus fixtures, 0
divergences. What is now at parity, layer by layer: the **structural body walk + CFN
skeleton + ids + routine/object metadata** (R1a — operations, call-sites incl.
`ExpressionInfo` + result-use, record-ops, loops, vars, var-assignments, condition
refs, field accesses, `nestingDepth`/`hasBranching`/`identifierReferences`/
`unreachableStatements`, normalized CFN); the **control-context lattice** (R1b —
`controlContext` per op/callsite, IsHandled elevation, TryFunction guard, error-call
post-pass); the **operation-order + scope frames** (R1c — `order`/`scopeFrames` with
`onSuccessPath`/`dominatesSuccessReturn`); and the **direct capability facts** (R1d —
the 13 extractors' output, L3 identity stripped, publish excluded).

Three disciplines carried the gate and are worth keeping for R2:

- **The capture-point discipline.** Every sub-gate captures al-sem at a PRECISE
  boundary and replicates ONLY what is determinable there. R1a/b/c capture the
  pre-resolve index; R1d captures `extractCapabilities` on pre-resolve routines (NOT
  the L4 summary), replicating only the L2 opaque override and excluding the
  L4-injected publish facts + the L4-augmented coverage. Getting the boundary wrong
  (capturing L4 state at L2) is the single biggest source of false divergence — the
  Rev-2 review caught exactly this for publish + status/reasons.
- **The determinism contract.** Every derived collection has a canonical, explicit
  sort (objects by StableObjectId, routines by StableRoutineId, capability reasons by
  the serialized kebab string — NOT enum declaration order — diagnostics by
  `(sourceRef, message)`); Map/Set iteration never leaks into output unsorted. The
  byte-for-byte differential rests on this.
- **A native L2-DIRECT oracle per sub-gate** — `l2cc_oracles.rs` (control-context
  lattice invariants), `l2order_oracles.rs` (ordering/scope-frame invariants),
  `l2cap_oracles.rs` (the capability contract). Each is ground-truth-free and
  asserts the L2 invariants DIRECTLY (not the downstream L4 effects the TS soundness
  oracles add), so it catches a class of regressions the finite corpus + the golden
  vectors miss — and, because the corpus differential is byte-parity with al-sem, a
  structural oracle failure would mean BOTH engines are wrong (flagged loudly in each
  oracle).

---

## R2a parity status (SHIPPED — L3 record-type resolution, source-only) — R2's FIRST sub-gate

R2a is the FIRST sub-gate of R2: it establishes the **L3 foundation** (the
workspace symbol table + the post-resolve capture boundary) and ports
**record-type unification** — the resolved `tableId` R1 deliberately stripped.
The full source-only `ws-*` corpus differential is **153/153 green** with the
resolved record-var/op `tableId` + merged TableExtension fields compared;
`KNOWN_DIVERGENCES.json` is empty; the anti-degenerate coverage matrix is healthy.

Key facts of the port (all at parity with the TS oracle):

- **Capture point = POST-RESOLVE / PRE-SUMMARY.** The dump runs `indexWorkspace`
  (L2) → `resolveModel` (L3) and captures the RESOLVED model BEFORE any L4 summary
  / combined-graph / `composeSnapshot`. `tableId` is set by `resolveRecordTypes`
  and never re-touched by `mergeExtensionFields` or later steps, so the capture is
  clean. The Rust side runs ONLY the first three resolve sub-steps
  (`build_symbol_table → resolve_record_types → merge_extension_fields`); calls /
  events / coverage are LATER gates (R2b/R2c/R2d) and OUT of R2a. The differ
  HARD-FAILS if any later-gate / L4 field (`callGraph` / `eventGraph` / `coverage`
  / `typedEdges` / `resourceId` / `bindingResolution` / `argumentBindings` /
  `summary` / `capabilityFactsDirect`) appears on either side.
- **Comparison surface = StableTableId.** Per record var/op, the resolved `tableId`
  is projected as a **StableTableId** (`${appGuid}:Table:${number}`) — never the
  internal `/` form, never the raw modelInstanceId-bearing id. An UNRESOLVED
  tableId (table not in the workspace symbol table) is **ABSENT** (omitted),
  matching al-sem (never guessed). Per Table object, the **merged extension fields**
  (TableExtension fields merged into the base table: field number/name/dataType/
  fieldClass + StableObjectId declaring provenance) are also compared.
- **DETERMINISTIC INGESTION ORDER is load-bearing.** Collision resolution is
  order-dependent: the symbol table's name/number indexes are LAST-wins, the
  record-op lexical-scope fallback (`variablesByName`) is LAST-wins, and
  `mergeExtensionFields` is FIRST-wins. The Rust side assembles the workspace L3
  model in al-sem's EXACT ingestion order — POSIX-path-sorted files → per-file
  document order — so every collision resolves identically. The
  `ws-r2a-record-types` fixture pins this (two TableExtensions colliding on field
  50000 → the first-ingested wins).
- **Implicit `Rec`/`xRec` via the effective own table.** A still-unset implicit
  `Rec`/`xRec` op resolves to its object's effective own table: a Table → itself; a
  Page → its `SourceTable`; a TableExtension → its `extends` target; a PageExtension
  → the base page's `SourceTable` (via the extends chain). An explicit local `Rec`
  variable is NEVER overridden by this pass.
- **The anti-degenerate COVERAGE MATRIX.** The differential computes + ENFORCES
  nonzero counts of resolved record-var tableIds, resolved record-op tableIds,
  implicit-Rec resolutions, and merged extension fields across the corpus (computed
  from the RUST output, so it proves resolution actually FIRES — a degenerate
  all-unresolved port would otherwise pass a pure equality diff), and cross-checks
  them against the GOLDEN (al-sem ground-truth) counts. Current full-corpus matrix:
  **resolvedRecordVarTableIds=228, resolvedRecordOpTableIds=281,
  implicitRecResolutions=76, mergedExtensionFields=9**.

**R2a's native soundness oracle is L3-DIRECT** (`tests/l3rt_oracles.rs`), NOT a
golden diff (that is `l3rt_vectors.rs` + the differential's `*.l3rt.golden.json`,
byte-parity with al-sem). It drives small inline single-app workspaces through the
real `assemble_and_resolve_default` and asserts the record-type CONTRACT directly
on the resolved model: a record var/op's `tableId` is present IFF the table name
resolves in the workspace symbol table (`Record "NoSuchTable"` → absent, `Record
Customer` in-workspace → present); an implicit `Rec` in a Table trigger resolves to
THAT table; an implicit `Rec` in a Page resolves via `SourceTable`; a TableExtension
implicit `Rec` resolves via the `extends` chain; an explicit local `Rec` is never
overridden by implicit resolution; a `temporary` record still resolves its table;
the extension-merge collision is FIRST-wins; and resolution is case-insensitive
(`record customer` ≡ `Record CUSTOMER`). 8 tests, all green, **no
`src/engine/l3/**` change required** — the resolution was correct. Because the
corpus differential is byte-parity, a structural oracle failure would mean BOTH
engines are wrong (flagged loudly in the oracle).

### Covered vs deferred (R2a — honest)

- **Covered (source-only intra-workspace record-types):** the resolved record-var /
  record-op `tableId` (declared vars → ops → lexical-scope fallback → implicit
  `Rec`/`xRec` via the effective own table) + the merged TableExtension fields, in
  StableTableId/StableObjectId form, captured post-resolve / pre-summary.
- **Deferred to R2.5 / later gates:**
  - **CROSS-APP record-types** — a `Record` whose table lives in a `.app` symbol
    package, not the source workspace. The R2a corpus is source-only (no `.app`
    ingestion), so a table absent from the source resolves to ABSENT; binding it to
    the dependency's TableId needs `.app` projection → **R2.5**.
  - **CALL graph + EVENT graph resolution** (callee binding, dispatch,
    `callsiteResolutions`, `typedEdges`, publisher↔subscriber edges) — **R2b /
    R2c**. R2a runs only the first three resolve sub-steps.
  - The L3-resolved capability `resourceId` upgrade + `confidence` literal/enum →
    `"static"` (R2d), and the L4 summary / inherited-cone facts.

---

## R2b (L3 call graph) — STUB

R2b is the next sub-gate after R2a: the **call graph** — `resolveCalls` binding
each L2 call-site to its resolved target routine(s) across the workspace. It
**reuses the R2a workspace symbol table** verbatim (the symbol table keys routines
by `${objectId}::${name.toLowerCase()}` and pre-sorts overload lists by id — locked
in R2a precisely so R2b's overload resolution rests on a stable key + sort), and
DEPENDS on R2a's resolved `tableId` (overload disambiguation /
`inferRecordFieldType` needs the receiver's resolved table). Scope:

- **Overload disambiguation.** Pick the target overload from the call-site's
  argument types — keyed off R2a's resolved record-var / field types (strict
  prereq).
- **Dispatch kinds.** Member / interface / object-run (`Codeunit.Run`) dispatch;
  per-callsite `dispatchKind` + `resolution`. **Interface dispatch is MULTI-edge**
  — one call-site fans out to EVERY implementer (don't collapse by `callsiteId`).
- **The resolved binding fields R1/R2a keep forbidden.** `upgradeBindings`
  (`argumentBindings` upgraded by the resolver), `callsiteResolutions`,
  `calleeParameterIsVar` / `bindingResolution` / `sourceTableId`.
- **Port the scalar type machinery as exact tested units FIRST.** The
  `type-relation` / `normalizeAlType` regexes (scalar type normalization +
  assignability) must be ported as standalone, vector-tested units BEFORE the call
  graph consumes them — the same discipline that carried R1's encoders.
- Pin opaque-vs-external dispatch with the declared-dep-no-`.app` fixture.

R2b extends, never replaces, the R1/R2a goldens — the validated L2 surface + the
resolved record-type identity are the inputs the call-graph resolver binds.

---

## Running migration status

| Layer | Gate(s) | Status |
| --- | --- | --- |
| L0 (identity) | R0 | **DONE** — 157/157 source-only corpus, allowlist empty |
| L2 (index) | R1 (R1a+R1b+R1c+R1d) | **DONE** — 152/152, full per-routine feature surface |
| L3 (resolve) | **R2a** (record-types, source-only) | **DONE** — 153/153 + coverage matrix + native oracle |
| L3 (resolve) | R2b (call graph), R2c (event graph), R2d (coverage/gaps) | remaining |
| L3 (resolve) | R2.5 (`.app` ingestion + cross-app resolution) | remaining |
| L4 (summaries) | R3 | not started |
| L5 (detectors) | R4 | not started |
| product | — | not started |

With R2a shipped, **R0 + R1 + R2a are done**. R2b / R2c / R2d + R2.5 (`.app` /
cross-app) remain for the L3 resolve layer; then R3 (L4 summaries), R4 (L5
detectors), and the product surface.
