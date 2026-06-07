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
| **R2b** | L3 call graph (source-only): every call-site resolved to `CallEdge[]` (`from`/`to?`/`callsiteId`/`operationId`/`dispatchKind`/`resolution`/`candidates?`/`externalTypeRef?`/`receiverType?` + GROUP-level `dispatchMeta`) — overload disambiguation, MULTI-edge interface dispatch, object-run, opaque/external-target — plus the `upgradeBindings` argument-binding upgrade + implicit-trigger edges, captured POST-RESOLVE / PRE-SUMMARY in stable-id form | **SHIPPED** — 155/155 fixtures with grouped multiset-compared CallEdges + group dispatchMeta + upgraded argumentBindings, 0 divergences, the expanded anti-degenerate coverage matrix healthy + manifest-oracle-equal (direct=123, member=26, objRun=4, ifaceMultiEdge=2, ifaceEdges=6, dynamic=3, builtin=18, implicit=6, unresolved=33, ambiguous=4, memberNotFound=1, opaque=5, external=4, upBind=54, ambBind=1), native L3-direct call-graph oracle green |
| **R2c** | L3 event graph (source-only): `buildEventGraph` binding each `[EventSubscriber]` to its publisher(s) — `EventSymbol[]` (real `[IntegrationEvent]`/`[BusinessEvent]` publishers + synthesized maybe/unknown symbols; `eventKind`/`isolated`/`signatureHash`/`elementName`) + `EventEdge[]` (open-world: every parseable subscriber → exactly one edge; `resolution` = `resolved`/`maybe`/`unknown`) — with the FIXED `realPublisherEventIds` resolution semantics, captured POST-RESOLVE / PRE-SUMMARY in stable-id form | **SHIPPED** — 31/31 fixtures with stable-projected `events[]`+`edges[]` compared, 0 divergences, `KNOWN_DIVERGENCES` empty, the anti-degenerate coverage matrix healthy + manifest-oracle-equal (integrationPub=40, businessPub=5, unknownKind=8, isolatedPub=5, elementName=1, resolved=41, maybe=7, unknown=3), native L3-direct event-graph oracle green (8 invariants); the 6th al-sem oracle bug (false-`resolved`) fixed in the FIXED semantics |
| **R2d** | L3 coverage (source-only): `buildCoverage` — the "no silent clean" `AnalysisCoverage` accounting: `sourceUnitsTotal`/`sourceUnitsParsed` (the index-stage-warning `failedUnitRefs` decrement — corpus-inert, vector-covered), `routinesTotal`/`routinesBodyAvailable` (count) / `routinesParseIncomplete` (StableRoutineId[], INDEPENDENT filters — NOT a partition), `opaqueApps` (empty source-only), `unresolvedCallsites` (StableCallsiteId MULTISET: the 4 resolutions unknown/ambiguous/member-not-found/external-target — NOT opaque/builtin/maybe; duplicates PRESERVED), `dynamicDispatchSites` (StableOperationId MULTISET: dispatchKind=="dynamic"), captured POST-RESOLVE / PRE-SUMMARY in stable-id form, READ off the parity R2b call graph + L2 routine flags | **SHIPPED** — 158/158 fixtures with the projected `AnalysisCoverage` compared (multisets positional after sort, dups preserved), 0 divergences, `KNOWN_DIVERGENCES` empty, the anti-degenerate coverage matrix healthy + manifest-oracle-equal (sourceUnitsTotal=314, sourceUnitsParsed=314, routinesTotal=567, routinesBodyAvailable=567, routinesParseIncomplete=1, opaqueApps=0, unresolvedCallsites=86, dynamicDispatchSites=3; unresolvedMaxDup=1, dynamicMaxDup=1), native L3-direct coverage oracle green (5 invariants) |
| **R2 (= R2a+R2b+R2c+R2d)** | Full **source-only L3 resolve** parity: record types + call graph + event graph + coverage at the `resolveModel` (post-resolve / pre-summary) boundary | **COMPLETE** — all four sub-gates SHIPPED; the full source-only L3 surface is at byte-parity with al-sem over the corpus + a native L3-direct oracle per sub-gate. NEXT: **R2.5** (`.app` symbol reader → cross-app L3, where `opaqueApps` + the real unfetched-dep coverage become non-empty) |

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

## R2b parity status (SHIPPED — L3 call graph, source-only) — R2's SECOND sub-gate

R2b is the SECOND sub-gate of R2: the **call graph** — `resolveCalls` binding each
L2 call-site to its resolved target routine(s) across the workspace. It **reuses the
R2a workspace symbol table** verbatim (routines keyed by
`${objectId}::${name.toLowerCase()}`, overload lists pre-sorted by id — locked in
R2a precisely so R2b's overload resolution rests on a stable key + sort) and DEPENDS
on R2a's resolved `tableId` (arg-type overload disambiguation reads the receiver's
resolved record-var / field table). The Rust port lives in
`src/engine/l3/call_resolver.rs` (`resolve_calls` / `resolve_call_site` /
`resolve_by_name_and_arity` / `resolve_interface_dispatch` / `upgrade_bindings`) +
the scalar primitives (`al_type` / `type_ref` / `type_rel` / `static_arg`) ported and
vector-tested FIRST, with the golden-shaped projection in
`src/engine/l3/call_graph_projection.rs`.

**Capture point = POST-RESOLVE / PRE-SUMMARY (READ the resolved model).** al-sem's
dump runs `indexWorkspace → resolveModel` ONCE and READS `model.callGraph` + the
in-place-upgraded `argumentBindings` — it NEVER re-runs `resolveCalls` /
`upgradeBindings` (`upgradeBindings` is non-idempotent: a second pass detects a
"double-upgrade" and emits a diagnostic + skips). The Rust emitter mirrors this:
`assemble_and_resolve_workspace → project_call_graph` builds the symbol table and runs
`resolve_calls` exactly once over a freshly-constructed per-callsite binding state, so
the read-resolved capture is reproduced (no re-entrant upgrade).

**Comparison surface (R2b).** Per callsite, the `CallEdge[]` GROUP — a callsite has
0..N edges; interface dispatch is MULTI-edge — `from` / `to?` / `operationId` /
`dispatchKind` / `resolution` / `candidates?` / `externalTypeRef?` / `receiverType?`,
all ids in STABLE form. `dispatchMeta` (`interfaceName` / `totalImpls` /
`unresolvedImpls` / `enumImplementers` — **NO `openWorld`**, which does not exist in
shipped source) is projected at the **GROUP level**: the resolver attaches it to one
edge only (an internal-RoutineId sort artifact), so group-level projection makes the
multi-edge comparison order-robust. Interface RESOLVED impl edges carry
`resolution: "maybe"` (NOT "resolved") — the coverage matrix counts "interface
multi-edge" independently of `resolution == "resolved"`. Plus the UPGRADED
`argumentBindings` per callsite (`parameterIndex` / `calleeParameterIsVar` /
`bindingResolution`, all bindings whose callsite has ≥1 binding — `non-record-arg`
included). Implicit-trigger edges (Rec op → table trigger) appear in their own groups.

**The differ groups `callsiteId → Vec<CallEdge>` and compares each group as a SORTED
MULTISET** (`tests/differential.rs::diff_l3cg`) — never `Map<callsiteId, CallEdge>`,
so a multi-edge interface callsite is compared edge-for-edge. The within-group edge
sort + the 5 other sort points (candidates / unresolvedImpls / enumImplementers /
groups / bindings) use ONE byte-order comparator on the projected stable strings,
identical to al-sem's `cmpStable` + `edgeSortKey`. The ws-d2 smoke is **byte-identical**
to the golden. **DROPPED (plan Rev 2 #5):** `callsiteResolutions` — not certified
L3-clean. **FORBIDDEN (hard-fail):** `typedEdges` / `summary` / `coverage` /
`eventGraph` / `callsiteResolutions` / `openWorld` / `capabilityFactsDirect` /
`rootClassifications` — scanned on BOTH sides; structurally absent from the serde
types regardless.

The full source-only corpus differential
(`differential_l3_call_graph_match_goldens`, committed default `full`) covers all
**155** `tests/r2b-goldens/*.l3cg.golden.json` at **0 divergences**,
`KNOWN_DIVERGENCES.json` empty — and the R2b Task 3 resolver needed NO change to reach
full-corpus green (the resolution was already correct against the vectors).

**Anti-degenerate COVERAGE MATRIX (expanded `[REV2]`).** The differential counts +
ENFORCES nonzero per axis (full set only): resolvedDirect=123, resolvedMember=26,
objectRunResolved=4, interfaceMultiEdge=2, interfaceEdges=6, dynamicUnknown=3,
builtin=18, implicitTrigger=6, unresolvedUnknown=33, ambiguous=4, memberNotFound=1,
opaque=5, externalTarget=4, upgradedResolvedBindings=54, ambiguousBindings=1. Rust
coverage is asserted EQUAL to the golden coverage per run, and a dedicated oracle
(`l3cg_coverage_matrix_matches_manifest_oracle`) asserts the full-corpus Rust totals
EQUAL the al-sem manifest's published `coverageMatrix`.

**The `primaryDependencies`-empty dump-path nuance (member-opaque).** In the bare
`assemble→resolve→project` dump path `has_unfetched_declared_dependency` is ALWAYS
false (no `.app` deps fetched; `primary_dependencies` empty), so the member-call
"opaque" branch is structurally UNREACHABLE — every missing member object resolves to
`external-target` (this is why `ws-r2b-opaque`'s golden, generated by the same dump
path, shows `external-target`, exactly as its fixture comment warns). The `opaque`
axis is therefore populated SOLELY by OBJECT-RUN misses (a missing object-run target
is ALWAYS opaque), which the corpus DOES exercise (5 edges) — so `opaque` is still
enforced nonzero, and `external-target` is enforced as the plan requires. Binding a
member miss to `opaque` requires `.app` ingestion → **R2.5**.

**R2b's native soundness oracle is L3-DIRECT** (`tests/l3cg_oracles.rs`) — 8
ground-truth-free STRUCTURAL invariants over the resolved/projected graph, NOT a
golden diff: interface dispatch emits one edge PER resolved impl and the projection
NEVER collapses by callsiteId (a callsite carries >1 edge, all "maybe", group
dispatchMeta present); a resolved edge's `to` resolves to a real workspace routine;
an ambiguous overload → "ambiguous" with ≥2 byte-order-sorted candidates;
wrong-arity member → "member-not-found"; object-run resolves to `OnRun`; the
external-target (member miss, no dep) vs object-run-opaque distinction holds;
`upgrade_bindings` runs EXACTLY once (no double-upgrade diagnostic, while the binding
IS upgraded — so "no diagnostic" is non-vacuous); the within-group edge sort is
deterministic / byte-order stable. Because the differential is BYTE-PARITY, a failure
here the differential misses would mean BOTH engines are wrong.

### Covered vs deferred (R2b — honest)

- **Covered (source-only intra-workspace call resolution):** bare/direct calls,
  member dispatch (receiver-typed), the 3-tier overload resolution (name → arity →
  arg-type disambiguation off R2a's resolved tables), MULTI-edge interface dispatch +
  group dispatchMeta, object-run (`Codeunit.Run` / page-run / report-run → `OnRun` or
  first routine), the instance `<codeunitVar>.Run([Rec])` special-case, dynamic
  object-run, builtins, implicit-trigger edges, unresolved/member-not-found/ambiguous,
  `external-target` (member miss, fetched-complete), object-run `opaque`, and the
  one-time `upgradeBindings` argument-binding upgrade.
- **Deferred to R2.5 / later gates:**
  - **CROSS-APP opaque / cross-app call targets** — a member/object-run target in a
    `.app` symbol package (so `has_unfetched_declared_dependency` is TRUE and member
    misses become `opaque`). The R2b corpus + oracle are source-only (no `.app`
    ingestion) → **R2.5** (`.app` projection).
  - **EVENT graph** (publisher↔subscriber edges, `parseSubscriberAttribute`,
    `parseIsolated`, open-world synthetic event ids) → **R2c**.
  - **`callsiteResolutions`** — dropped from R2b (not certified L3-clean; may pull
    L4/typedEdges/compose state). Audit + add in a later phase if proven clean.
  - **REACHABILITY-crosscheck + inter-never-overclaim** soundness oracles — these need
    the COMBINED graph (call + event + implicit) and a reachability walk that R2b does
    not compute (R2b stops at the resolved call edges). They land where reachability is
    computed (the L4 / combined-graph gate), NOT here.
  - The L4 `typedEdges` / `RoutineSummary` / coverage(R2d) facts.

---

## R2c parity status (SHIPPED — L3 event graph, source-only) — R2's THIRD sub-gate

R2c is the THIRD sub-gate of R2: the **event graph** — `buildEventGraph` binding each
`[EventSubscriber]` to its publisher(s), reusing the R2a workspace symbol table
verbatim. It extends, never replaces, the R1/R2a/R2b goldens — the validated L2
surface + the resolved record-type identity + the resolved call graph are the inputs
the event resolver binds.

**What R2c reproduces.**

- **`publisherEventKind` / read-resolved capture** — every real publisher routine
  (`kind == "event-publisher"`) becomes one `EventSymbol`: `eventKind` =
  `[IntegrationEvent]` → integration, `[BusinessEvent]` → business; `signatureHash` =
  the routine's return-type-aware normalized signature hash; `parameters` carried in
  positional shape. The dump READS `model.eventGraph` post-resolve / pre-summary — it
  never re-runs `buildEventGraph`.
- **`parseSubscriberAttribute`** — decode the string-encoded `[EventSubscriber(...)]`
  args (`ObjectType::Codeunit`, `Codeunit::"Name"`, `'EventName'`, `'ElementName'` —
  qualified_enum_value / database_reference / string_literal) via the R1 structured
  `AttributeInfo` shape (AttributeInfo parity: the SAME L2 attribute indexing that
  produces the R1 projection's `attributesParsed`, so the arg shape cannot drift). A
  malformed / unparseable `[EventSubscriber()]` yields NO edge AND no synthesized
  symbol (open-world: every PARSEABLE subscriber → exactly one edge, no silent gap; a
  non-parseable one → none).
- **The FIXED `realPublisherEventIds` semantics** — a subscriber is `resolved` IFF a
  REAL indexed event-publisher routine produced its eventId, tracked in
  `real_publisher_event_ids` (NEVER in `event_by_id`, which also holds synthesized
  maybe/unknown symbols and drives ONLY dedup). Consulting `event_by_id` for the
  resolution decision was the **6th al-sem oracle bug** (false-`resolved`): it would
  upgrade the 2nd+ subscriber to an unindexed event on an existing object to
  `resolved`. FIXED: two such subscribers stay BOTH `maybe`.
- **Open-world three-case synthesis** — target found + real publisher → `resolved` (no
  synthesis); target found + NO real publisher → `maybe` + a synthesized symbol with
  the EXISTING target object's conforming objectId; target NOT found → `unknown` + a
  synthesized symbol with a NON-conforming sentinel objectId
  (`unknown/<type>/0:<targetRef>`). Every synthesized `signatureHash ==
  sha256Hex(RAW eventId)`; `encodeEventId` LOWERCASES the eventName (a mixed-case
  subscriber resolves). Note (MEMORY): a publisher-write ≺ subscriber-IO ordering can
  never be refutation-grade in open-world — binding-kind proves target-runs, not
  no-co-subscriber-commits.
- **Stable projection from EventSymbol** — `projectEventGraph` derives the stable
  eventId FROM the EventSymbol (dumb `/`→`:` on `publisherObjectId`, NEVER parsing the
  raw eventId; the sentinel `unknown/type/0:ref` → opaque `unknown:type:0:ref`), maps
  each edge's raw eventId through the rawEventId→stableEventId LAST-wins map, sorts
  events by stable id and edges by (stable eventId, subscriberRoutineId).

**Comparison surface (R2c).** The allowlisted event-graph projection (`events[]` +
`edges[]`) of a RESOLVED SemanticModel, byte-compared against the al-sem golden;
`forbiddenKeys` (callGraph/dispatch, typedEdges, summary, coverage, publish, …) guard
against later-gate leakage.

**R2c's native soundness oracle is L3-DIRECT** (`tests/l3eg_oracles.rs`) — 8
ground-truth-free STRUCTURAL invariants run NATIVELY over `build_event_graph` (inline
workspace → `assemble_and_resolve_default` → `SymbolTable::build` → `build_event_graph`),
asserting the event-graph CONTRACT in absolute terms (NOT a golden diff). Because the
corpus differential is BYTE-PARITY, a bug BOTH engines share would survive a pure
diff (that is exactly how the 6th false-`resolved` bug would hide) — the oracle catches
it. A failure here that the differential misses means BOTH engines are wrong — it is
NOT "fix the golden". The 8 invariants: open-world parseable→one-edge /
non-parseable→none; subscriber-to-real-publisher → resolved (no synthesis); two
subscribers to an unindexed event → BOTH maybe (the 6th-bug guard); target-found-no-
publisher → maybe + conforming synthesized symbol; target-not-found → unknown +
sentinel symbol; synthesized signatureHash == sha256Hex(raw eventId); publisher
eventKind tracks attribute; isolated parsing (true / explicit-false-omitted /
present-unparseable-conservative-true); eventName lowercased in the raw id. All green —
**no `src/engine/l3/event_graph.rs` change required** for the exit gate (the FIXED
semantics already landed in R2c Task 2; the oracle confirms it).

### Covered vs deferred (R2c — honest)

- **Covered (source-only intra-workspace event resolution):** real
  `[IntegrationEvent]`/`[BusinessEvent]` publisher symbols (eventKind, isolated,
  signatureHash, parameters); open-world subscriber edges (every parseable subscriber →
  exactly one edge; non-parseable → none); the FIXED `resolved`/`maybe`/`unknown`
  resolution; three-case open-world synthesis (conforming / sentinel objectId); the
  stable projection (dumb `/`→`:`, rawEventId→stableEventId LAST-wins map). 31/31
  corpus differential + coverage matrix + manifest oracle + native L3-direct oracle.
- **Deferred to R2.5 / later gates:**
  - **Cross-app event publisher / subscriber resolution** — a publisher or subscriber
    target that lives in a `.app` symbol package, not the source workspace. The R2c
    corpus + oracle are SOURCE-ONLY (no `.app` ingestion) → **R2.5** (`.app`
    projection) then cross-app.
  - **Publish-capability facts** — whether a publisher's transitive callee actually
    `Commit`s / raises before a subscriber observes is an effect-summary property the
    event graph does NOT compute (`op:"publish"` is L4-injected) → **L4 / R3**.

R2c extends, never replaces, the R1/R2a/R2b goldens — the validated L2 surface + the
resolved record-type identity + the resolved call graph are the inputs the event
resolver binds.

---

## R2d parity status (SHIPPED — L3 coverage, source-only) — R2's FOURTH + LAST sub-gate — R2 SOURCE-ONLY L3 COMPLETE

R2d is the FOURTH and LAST source-only L3 sub-gate: **`buildCoverage`** — the "no
silent clean" `AnalysisCoverage` accounting (`src/resolve/coverage.ts`, a 67-line pure
function ported to `src/engine/l3/coverage.rs`). It adds NO resolution — it is a pure
read over the already-parity R2b resolved call graph + the L2 routine flags, captured
POST-RESOLVE / PRE-SUMMARY (the dump runs `assemble→resolve→project_coverage_disk` and
reads `model.coverage` ONCE; it never re-runs `buildCoverage`).

**What R2d reproduces (the `AnalysisCoverage` surface, exactly).**

- `sourceUnitsTotal` = count of `kind:"source"` units; `sourceUnitsParsed` = that minus
  the `failedUnitRefs` set — units whose `id` is the `sourceRef` of an index-stage
  diagnostic with EXACTLY `stage:"index"` && `severity:"warning"` && `sourceRef`
  present (`info` does NOT decrement; the message is NOT consulted — `buildCoverage`
  keys ONLY on the triple). SOURCE-ONLY the decrement path is **INERT** (no fixture
  emits an index warning → `sourceUnitsParsed === sourceUnitsTotal` everywhere); the
  logic is implemented correctly and exercised by a `warning_unparsed` VECTOR (an empty
  `.al` file → al-sem's exact `Failed to index …` warning).
- `routinesTotal`; `routinesBodyAvailable` (COUNT of `bodyAvailable`);
  `routinesParseIncomplete` (the StableRoutineId[] of `parseIncomplete` routines).
  These two are **INDEPENDENT filters over the L2 flags, NOT a partition** — a
  syntax-error body that still has a `code_block` is BOTH `bodyAvailable` AND
  `parseIncomplete`. The Rust port threads `bodyAvailable`/`parseIncomplete` onto
  `L3Routine` computed the SAME way the L2 projection does (`find_code_block(routine).
  is_some()` / `routine.has_error()`) so the flags cannot drift.
- `opaqueApps` = the appGuid[] of `sourceKind:"symbol-only"` apps — **EMPTY source-only**
  (no dependency apps; becomes non-empty only in R2.5).
- `unresolvedCallsites` = a SORTED **MULTISET** (`.map` over call-graph EDGES, NOT unique
  sites): every edge whose `resolution ∈ {unknown, ambiguous, member-not-found,
  external-target}` → its StableCallsiteId. **Duplicates PRESERVED, never deduped**
  (an interface multi-edge callsite can emit the same id twice — `maybe`, excluded
  here, but the contract holds). `opaque` / `builtin` / `maybe` are EXCLUDED.
- `dynamicDispatchSites` = a SORTED MULTISET: edges with `dispatchKind:"dynamic"` → their
  StableOperationId. `dynamicDispatchSites` is `OperationId[]` and `unresolvedCallsites`
  is `CallsiteId[]` — NOT array-subsets of each other (a `/csN` id vs a `/opN` id).

**Comparison surface (R2d).** The allowlisted `AnalysisCoverage` projection. The
differential (`differential_l3_coverage_match_goldens`) compares it structurally over
all 158 goldens — multisets POSITIONAL after the projection's sort, so a cardinality
OR id divergence (incl. a missing/spurious duplicate) is caught. FORBIDDEN later-gate /
L4 keys (callGraph(R2b) / eventGraph(R2c) / typedEdges / summary / `analysisGaps` /
capability* / rootClassifications) HARD-FAIL the pass on either side. `KNOWN_DIVERGENCES`
empty. The anti-degenerate **coverage matrix** (driven by the RUST projection) enforces
nonzero `routinesBodyAvailable` / `routinesParseIncomplete` / `unresolvedCallsites` /
`dynamicDispatchSites`, asserts `opaqueApps == 0` + `sourceUnitsParsed == sourceUnitsTotal`,
and an oracle cross-check (`l3cov_coverage_matrix_matches_manifest_oracle`) asserts the
full-corpus totals equal the al-sem manifest `coverageMatrix` (incl. the
`unresolvedMaxDup` / `dynamicMaxDup` / `sourceUnitsDecremented` axes).

**`analysisGaps` is DROPPED from R2d** (Rev 2 MUST-FIX #3) — it derives `opaqueApps`
from body-unavailable DEPENDENCY routines + dep-app boundaries that
`withDependencyArtifacts` injects, so it is tied to the cross-app surface → revisited in
R2.5. The authoritative R2d surface is the raw `AnalysisCoverage` only.

**R2d's native soundness oracle is L3-DIRECT** (`tests/l3cov_oracles.rs`) — 5 STRUCTURAL
invariants run NATIVELY against the Rust coverage, RE-DERIVING the expected multisets
from the resolved edges + L2 flags rather than diffing al-sem strings: (1)
`unresolvedCallsites` == exactly the 4-resolution edge multiset + `dynamicDispatchSites`
== the dynamic-edge multiset, and the OperationId/CallsiteId id-spaces never overlap;
(2) builtin/opaque/resolved are NEVER in unresolved; (3) `bodyAvailable` (count) +
`parseIncomplete` (list) are independent (a routine BOTH); (4) `opaqueApps` empty +
no real duplicate (max-dup 1) source-only; (5) the sorted-multiset + duplicate-
preservation contract (synthetic, since no AL source produces a real unresolved dup).

### Covered vs deferred (R2d — honest)

- **Covered (source-only):** the full `AnalysisCoverage` — source-unit counts + the
  warning-decrement logic (vector-covered, corpus-inert), routine body/parse-incomplete
  accounting, the 4-resolution unresolved multiset + the dynamic multiset (dups
  preserved), `opaqueApps` empty. 158/158 corpus differential + coverage matrix +
  manifest oracle + native L3-direct oracle.
- **Deferred to R2.5 / later gates:**
  - **`opaqueApps` (non-empty)** + the real **unfetched-dependency coverage** — a
    symbol-only `.app` dependency app (`sourceKind:"symbol-only"`) is what makes
    `opaqueApps` non-empty and a member miss `opaque`. The R2d corpus + oracle are
    SOURCE-ONLY (empty `.app` ingestion) → **R2.5** (`.app` projection).
  - **`analysisGaps`** — the per-app gap derivation (body-unavailable DEP routines +
    dep-app boundaries) is tied to the cross-app surface → **R2.5**.
  - The L4 `typedEdges` / `RoutineSummary` / per-app `CoverageRecord` (compose) facts.

With R2d SHIPPED, **R2 (= R2a + R2b + R2c + R2d) is SOURCE-ONLY L3 COMPLETE**: record
types + call graph + event graph + coverage are all at byte-parity with al-sem over the
corpus, each with a native L3-direct oracle. R2d extends, never replaces, the
R1/R2a/R2b/R2c goldens.

---

## R2.5 (`.app` symbol reader + cross-app L3) — STUB

R2.5 is the FIRST cross-app gate — it lifts the SOURCE-ONLY restriction every L3
sub-gate (R2a–R2d) carried. Scope:

- **`.app` symbol reader** — port `symbols/symbol-reference-parser.ts` (~504 LOC): a
  `.app` package is a ZIP (read via the `zip` crate); the embedded `SymbolReference`
  JSON projects to the SAME `Routine` / `ObjectDecl` / `Table` / `Event` shape the
  native source path produces, via `deps/dependency-projection.ts`. Per CLAUDE.md's
  "Native + ABI must agree on model shape" — `RoutineId`, `attributesParsed`,
  `parameters`, `accessModifier`, `features.identifierReferences`, the canonical
  signature hash — so cross-app call resolution actually matches up.
- **Cross-app L3 gate** — re-run R2a–R2d over `workspace + deps`: cross-app member
  dispatch (a member miss on a typed receiver whose object lives in a fetched dep
  resolves; in an UNfetched dep → `opaque`), record-types vs dep tables, interface-impl
  completeness across apps, event-publisher lookup across apps, and — the R2d hook —
  `opaqueApps` + the real unfetched-dep coverage become **non-empty**, and
  `analysisGaps` (deferred from R2d) lands.

R2.5 reuses the resolved source-only L3 verbatim — it ADDS the dependency projection as
a second model-feed, never replaces the native one. It extends, never replaces, the
R1/R2a/R2b/R2c/R2d goldens.

---

## Running migration status

| Layer | Gate(s) | Status |
| --- | --- | --- |
| L0 (identity) | R0 | **DONE** — 157/157 source-only corpus, allowlist empty |
| L2 (index) | R1 (R1a+R1b+R1c+R1d) | **DONE** — 152/152, full per-routine feature surface |
| L3 (resolve) | **R2a** (record-types, source-only) | **DONE** — 153/153 + coverage matrix + native oracle |
| L3 (resolve) | **R2b** (call graph, source-only) | **DONE** — 155/155 + expanded coverage matrix + manifest oracle + native L3-direct oracle |
| L3 (resolve) | **R2c** (event graph, source-only) | **DONE** — 31/31 + coverage matrix + manifest oracle + native L3-direct oracle (8 invariants); 6th al-sem oracle bug (false-`resolved`) fixed |
| L3 (resolve) | **R2d** (coverage, source-only) | **DONE** — 158/158 + coverage matrix + manifest oracle (incl. max-dup/decremented axes) + native L3-direct oracle (5 invariants) |
| L3 (resolve) | **R2 (= R2a+R2b+R2c+R2d)** | **SOURCE-ONLY L3 COMPLETE** — full source-only L3 surface at byte-parity + a native L3-direct oracle per sub-gate |
| L3 (resolve) | R2.5 (`.app` ingestion + cross-app resolution) | remaining — the FIRST cross-app gate (`opaqueApps`/`analysisGaps` become non-empty) |
| L4 (summaries) | R3 | not started |
| L5 (detectors) | R4 | not started |
| product | — | not started |

With R2d shipped, **R0 + R1 + R2a + R2b + R2c + R2d are done** — **R2 is SOURCE-ONLY L3
COMPLETE** (record types + call graph + event graph + coverage all at byte-parity over
the corpus, each with a native L3-direct oracle). NEXT: **R2.5** (`.app` symbol reader +
cross-app L3, where `opaqueApps` + the real unfetched-dep coverage + `analysisGaps`
become non-empty); then R3 (L4 summaries), R4 (L5 detectors), and the product surface.
