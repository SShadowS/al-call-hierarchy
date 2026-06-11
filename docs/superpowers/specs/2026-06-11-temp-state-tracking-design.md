# Temp-state tracking epoch — structural capture + substitution + path-time resolution — Design

> **Context:** Real-world validation on a shipping AppSource app (Continia Document Output v29)
> surfaced a false-positive class: a codeunit MEMBER variable `Files: Record "CDO File" temporary;`
> was not recognized as temporary — d33 (unfiltered-bulk-write) fired **critical** on
> `Files.DeleteAll()` (an in-memory buffer clear), and d1 stamped member-var ops
> "(temp state uncertain)". Root cause is a half-finished temp-state substrate: al-sem's
> `TempState = Known(bool) | ParameterDependent(i) | Unknown` works for procedure locals and
> parameters but leaks everywhere else. This epoch completes it. Scope chosen by the user:
> **full, including ParameterDependent substitution** — best solution, time not a constraint.
>
> **POST-FLIP, RUST-ONLY (supersedes the original dual-repo plan):** the TS oracle was retired on
> 2026-06-11 (see `docs/engine-migration.md` "FLIPPED"). This epoch is implemented ONLY in the Rust
> engine (`src/engine/`). The al-sem TS file:line references throughout this spec are FROZEN-ORACLE
> reference semantics (the behavior being completed), not implementation sites. The Rust
> implementation sites are mapped in "Implementation & baseline plan (post-flip)" at the end —
> which replaces the original "Migration / parity plan" section. Goldens are rebaselined from this
> engine with reviewed diffs; `KNOWN_DIVERGENCES.json` stays `[]` (the concept is retired).

## Goal

Temp-record knowledge captured at every declaration site, propagated soundly through the call
graph, and resolved exactly per evidence path — so detectors suppress/downgrade on genuinely
temporary records (no false criticals on in-memory buffers) while every uncertain case keeps
firing conservatively.

## Verified current state (exploration, file:line)

Works today:
- **Locals**: `intraprocedural-refs.ts:154-203` reads `temporary_keyword` structurally →
  `RecordVariable.tempState = Known(bool)`.
- **Parameters**: `intraprocedural-refs.ts:96-151` — keyword → `Known(true)`; by-var no keyword →
  `ParameterDependent(index)`; by-value → `Known(false)`.
- **Backfill**: `routine-indexer.ts:277-292` copies the declared var's tempState onto each
  `RecordOperation` by receiver-name match (map of locals+params ONLY).
- **Consumers**: d1/d3/d5/d10/d18/d33/d36/d37/d40/d49 all gate on
  `tempState.kind === "known" && value === true` (suppress or downgrade-to-info). d33 suppression
  at `d33-unfiltered-bulk-write.ts:79`; d1 severity/note at `d1-db-op-in-loop.ts:81-159`.

Broken/missing (the gaps this epoch closes):
- **G1 — member/global record vars**: `variable-indexer.ts:25-64` (`extractObjectGlobals`)
  produces `VariableSymbol` with NO tempState field; the `temporary` keyword survives only as raw
  text inside `declaredType`. Member-var ops fall to `Unknown` at `routine-indexer.ts:285`.
- **G2 — half-finished L3 fallback**: `record-types.ts:65-85` backfills `tableId` from the
  unified variable list (includes globals) but never tempState — the asymmetry that produced the
  CDO finding.
- **G3 — `TableType = Temporary` tables**: `Table` entity (`entities.ts:201-208`) has no flag;
  the property is never read. Every record of such a table is temp regardless of var modifier.
- **G4 — page `SourceTableTemporary`**: not modeled; implicit `Rec` ops in such a page are temp.
- **G5 — ParameterDependent never substituted**: `summary-runner.ts:171-221` inherits callee
  effects with their original tempState. A `PD(i)` carried into a caller's summary references a
  *callee* param index — a latent incoherence (meaningless in the caller's frame). The callsite
  binding even captures `sourceTempState` (`intraprocedural-body.ts:238`) — it is simply never
  used.
- **G6 — RecordRef**: `RecRef.GetTable(TempRec)` tempness not propagated.
- **G7 — ABI side**: no evidence the `.app` symbol path captures parameter `temporary`
  (native+ABI shape-parity rule, CLAUDE.md).
- **G8 — zero test coverage** for any non-local temp state (all fixtures use locals).

## Architecture

Layered substrate fix. Detectors stay pure consumers — **zero detector logic changes**:

```
L0/L2  capture     temporary_keyword read STRUCTURALLY at every declaration site
L3     resolution  op.tempState backfill: table-level override → var declaration → PD/Unknown
L4     composition ParameterDependent SUBSTITUTED at effect inheritance (per callsite binding)
L5     path-walk   PD resolved exactly along the concrete evidence path → final severity
```

**One precedence rule everywhere:** table-level temp (`TableType = Temporary`) ⇒ `Known(true)`
regardless of var modifier; else var-declaration state; else PD/Unknown.

## Component 1 — structural capture

### 1a. Member/global record vars (G1, G2)
- `extractObjectGlobals` (`variable-indexer.ts`) parses `record_type → temporary_keyword` off the
  AST node (the same walk locals use at `intraprocedural-refs.ts:163`). `VariableSymbol` gains
  `tempState?: TempState`, set only for record-typed globals.
- Routine-indexer **promotes object-global record vars into each routine's `recordVariables`**
  (per-routine `RecordVariable` instances; id via the existing `encodeRecordVariableId(routineId,
  name)`; a `scope` marker distinguishes promoted globals). The existing backfill at
  `routine-indexer.ts:277-292` then resolves member-var ops with no new mechanism. The
  `record-types.ts` tableId fallback remains as a safety net; it additionally backfills tempState
  from the promoted entries (closing G2's asymmetry).
- **Shadowing (AL scoping):** a local/param shadows a same-named global. Backfill-map insertion
  order: globals first, locals/params overwrite → lookup yields the innermost declaration.

### 1b. `TableType = Temporary` (G3)
`Table` entity gains `isTemporary: boolean`, captured at table indexing from the property (native
parse). Applied at op resolution as the override rule. `.app` side: verify whether symbol packages
carry the property; if absent, document as an ABI gap and default `false` (= conservative — never
wrongly suppresses).

### 1c. Page `SourceTableTemporary` (G4)
Page entity gains the flag. Implicit `Rec`/`xRec` receiver ops inside that page object's routines
resolve to `Known(true)` — extends the existing implicit-Rec resolution at
`record-types.ts:87-136`.

### 1d. ABI parameters (G7)
Verify the symbol-reference parameter metadata for a `temporary` marker; capture it so cross-app
routines produce the same `Known(true)/PD/Known(false)` shapes as native (native+ABI parity rule).
If the marker is absent from the symbol format: by-value → `Known(false)`, by-var → `PD(i)` —
identical to native rules minus the keyword case; document.

### Rejected: string-sniffing
A regex `\btemporary\b` over `declaredType` is **forbidden**: it matches inside quoted table names
(`Record "My temporary stuff"`) → wrong `Known(true)` → silently suppresses a real critical.
Suppression-direction signals must be structural (AST keyword / object property), never textual.

## Component 2 — ParameterDependent substitution at L4 (G5)

`PD(i)` is a symbolic state whose meaning is "substitute the caller's binding." Today it is
created and never substituted; inherited effects carry foreign-frame indices. This component makes
composition coherent.

At `composeRoutineCtx` (`summary-runner.ts`), per **callsite** (not per merged edge — one caller
may call the same callee twice with different arguments), for each inherited callee effect with
`tempState = PD(i)`, look up the callsite's argument binding for callee param `i`:

| binding of arg `i` | inherited effect tempState |
|---|---|
| caller's known-temp var (incl. promoted member vars, table-level override) | `Known(true)` |
| caller's known-physical var | `Known(false)` |
| caller's own by-var param `j` | `PD(j)` — re-symbolized into the caller's frame, chains upward |
| unbindable expression / no binding (dynamic dispatch, event edge) | `Unknown` |

- One inherited effect per distinct substitution result. `effectKeyOf` already includes tempState
  (`summary-engine.ts:216-237`) → identical results dedupe, divergent results stay distinct —
  **"mixed" paths emerge as two effects**; no fourth lattice value.
- **Monotone + convergent:** substitution maps finite state → finite state at inheritance only;
  SCC/recursion handled by re-symbolization — a PD chasing itself around a cycle never *gains*
  `Known`, stabilizing as PD/Unknown within the existing finite fixed-point.
- **Open world honest by construction:** an event-subscriber's own summary keeps its PD; event
  edges carry no argument bindings → inherited `Unknown` → conservative. No closed-world gate.
- **Soundness direction:** substitution only narrows symbolic → binding-derived; all uncertainty
  falls to `Unknown` (fires). Suppression remains gated exclusively on `Known(true)`.

## Component 3 — path-time resolution at L5

The shared path-walker holds the concrete edge chain for each finding. Resolve the terminal op's
tempState exactly along it: op `PD(i)` → step one frame toward the path root via that callsite's
binding (same substitution table) → repeat until `Known`/`Unknown` or the path root (still PD at
the root = entry parameter, caller unknown → `Unknown`). Detector policies (d1 severity + note,
d33 suppression, etc.) read the **path-resolved** state: the caller-A path reports
temp/info while the caller-B path fires on the same op — per-finding truth, no approximation.

## Component 4 — RecordRef (G6, scoped small)

`RecRef.GetTable(SomeRec)` → the RecordRef variable's subsequent ops inherit `SomeRec`'s
tempState, **locally determinable only** (same routine, unconditional flow); anything beyond →
`Unknown`. `RecRef.Open(no, true)` (the OpenTemporary form) → `Known(true)`; plain `Open` →
`Known(false)`. RecordRef modeling otherwise unchanged.

**Out of scope (with rationale):** `Copy(..., ShareTable)` aliasing — AL tempness is
declaration-bound; assignment never transfers it, so plain data flow is not a tempness hazard.
ShareTable is the single odd corner and is rare; revisit only if real-world FPs surface.

## Detector policy

No detector logic changes. All consumers already gate on `Known(true)`:
- d33: the CDO `ClearFiles` critical disappears via the existing suppression
  (`d33-unfiltered-bulk-write.ts:79`); its `skippedTempRecord` counter now also counts member-var
  suppressions.
- d1: member-var temp ops downgrade to info with the existing honest note "(temporary record — not
  a SQL round-trip)"; "(temp state uncertain)" becomes rare and accurate.
- d3/d5/d10/d18/d36/d37/d40/d49 sharpen automatically.

## Error handling

Engine never throws — unchanged. Every parse/capture failure → field absent → `Unknown` →
conservative firing. No new diagnostic categories; coverage reporting untouched.

## Testing

- **Fixtures (one per new state):** member temp + member non-temp; `TableType = Temporary`;
  `SourceTableTemporary` page; PD upgrade chain (temp caller → by-var helper); **mixed callers**
  (temp + physical → two effects, two path verdicts); recursion through PD; event-subscriber PD
  stays unresolved; RecordRef `GetTable`; **local-shadows-global**.
- **Metamorphic soundness oracle** (`test/soundness/`, probe-harness style): for each fixture,
  adding `temporary` to any record declaration may only ever **remove or downgrade** findings,
  never add one — a mechanical guard for the suppression direction across the whole epoch.
- **Real-world acceptance:** rerun on the CDO workspace
  (`U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud`) — the `ClearFiles` d33 critical is gone;
  member-var d1 findings drop to info with the temp note.
- Per-layer unit tests (capture, promotion, shadowing, substitution table, path resolution);
  e2e snapshots rebaselined.

## Implementation & baseline plan (post-flip — replaces the original "Migration / parity plan")

The TS oracle is retired; this epoch lands ONLY in `src/engine/`. The Rust implementation sites
(verified by the adversarial reviewers against the actual port):

1. **Capture (Component 1):** `src/engine/l2/scope.rs` — `extract_object_globals` exists
   (~:242-303) but captures NO temp state on globals; add it (structural `temporary_keyword` read,
   same node walk as locals). `src/engine/l3/l3_workspace.rs` — promote object-global record vars
   into each routine's `record_variables` (globals-first/locals-overwrite shadowing order).
   `PRecordVariable` (`src/engine/l2/features.rs:220-226`) gains the optional `scope` marker;
   `Table` model gains `is_temporary` (read `TableType` property natively; read the
   `{"Name":"TableType","Value":"Temporary"}` property on the `.app` symbol path —
   `src/engine/deps/symbol_reference.rs`); page `SourceTableTemporary` flag; ABI parameters read
   `TypeDefinition.Temporary` (verified present in real `.app` packages).
2. **The first-wins fix (RV-5) FIRST:** `src/engine/l3/record_types.rs` — the self-documented
   LAST-wins lexical-fallback bug (line 8) flips to first-wins BEFORE tempState backfill lands
   there.
3. **Substitution (Component 2):** `src/engine/l4/summary_runner.rs` — the effect-inheritance edge
   loop (~:373-395) + the argument-bindings loop (~:513) provide the join points; the substitution
   table per RV-7 (incl. the parameter-source resolution and the edge-kind table).
   `TempStateKind::ParameterDependent(u32)` is already in the effect key
   (`src/engine/l4/effect_lattice.rs:81-94`); dedupe inherited effects by
   `(operationId, resolvedTempState)` per the RV-7 cardinality bound.
4. **Path-time resolution (Component 3):** the shared L5 path-walker gains
   `resolve_temp_along_path` per RV-6; d1 consumes it; the merge-tie rule (worst severity wins).
5. **The CalcFields/FlowField gate (RV-1):** `field_class` is already modeled on both native and
   ABI sides; gate the temp downgrade on every named field argument resolving non-FlowField.
6. **Baselines:** this epoch CHANGES finding content by design. Affected golden suites (the
   tempState-bearing r0/r1a/r2a/r2b/r2c/r2d/r2.5b/r3a*/r4/cli-* files, the digest tempState merge,
   the R3a-2 trace-oracle baselines) are REBASELINED from this engine: add a Rust-side regen path
   (env-gated, mirroring the retired `AL_SEM_DIR` refresh pattern), regenerate, REVIEW the diff
   finding-by-finding (the review IS the verification — every moved golden must be explainable by
   a designed semantic change), commit. The cache-version tuple (`symbolReader` "17"→"18") bumps in
   the Rust constants + the cli_c tuple test.
7. **Fixtures + oracle:** the fixture list (incl. by-value-of-temp, CalcFields-FlowField-on-temp,
   CalcFields-Blob-on-temp, shadowing, mixed callers, recursion-through-PD, event-subscriber-PD)
   and the metamorphic soundness oracle (RV-2's carved property) are Rust integration tests under
   `tests/`.
8. **Acceptance:** rerun `alsem analyze` on the CDO workspace
   (`U:\Git\DO.Support-SlowDOSetup\DocumentOutput\Cloud`) — the `ClearFiles` d33 critical is gone;
   `LoadFiles`' CalcFields-on-Blob drops to info; a synthetic CalcFields-on-FlowField-on-temp case
   keeps firing.
9. Known interaction: the frozen `--with-evidence` / inventory schemas (al-perf P3.2/P4 consumers)
   are SHAPE-stable; this epoch changes finding *content*, not schema — al-perf's stub-based tests
   are unaffected; its real-binary smokes assert parse-ability, not finding counts.

## Non-goals

ShareTable aliasing (rationale above). A "mixed" lattice value (emerges as distinct effects).
Context-sensitive whole-summary cloning (substitution-at-composition achieves per-path truth at a
fraction of the cost). New detectors (substrate only). Changing the default conservative behavior
of `Unknown`.

## Self-review notes

- The suppression-direction asymmetry (memory: additive-vs-suppressive) is the governing soundness
  rule: every new `Known(true)` source is syntax/property-exact; every uncertainty path lands on
  `Unknown`-fires; the metamorphic oracle mechanically enforces the direction.
- The substitution component fixes a latent incoherence (foreign-frame PD indices) — a correctness
  repair, not just a precision feature.
- Dual-repo lockstep is the dominant cost; the design deliberately changes no serialized schema
  shapes beyond the internal model + finding content, so the blast radius is goldens + snapshots,
  not consumer contracts.

---

## Revision 2 — folded from the three-reviewer adversarial pass (2× opus + gemini-3.1-pro) + empirical verification

The design body above is SUPERSEDED where it conflicts with this block. Reviewer claims were
verified against actual code, the actual tree-sitter-al grammar, a REAL `.app` symbol package
(Continia Core 29.0), and the REAL AL compiler (alc 18.0.35, runtimes 15.0 and 17.0, compiled
against Microsoft 28.0 symbols). Implement to THIS revision.

### RV-1 — CalcFields on temp records still hits SQL when FlowFields are involved (MUST) [gemini — the headline]
A temporary record's FlowField is computed by evaluating its `CalcFormula` against the (physical)
flow-target tables — a REAL SQL query per call, host tempness irrelevant. Blob loads on a temp
record ARE in-memory. Therefore d1's blanket "temp ⇒ info, not a SQL round-trip" is WRONG for
`CalcFields`/`SetAutoCalcFields`: it would fix the CDO d33 FP while creating a NEW false-negative
class in the same function family. **Policy:** an op `CalcFields`/`SetAutoCalcFields` on a
known-temp record downgrades ONLY when EVERY named field argument resolves (via the table model)
to `fieldClass !== "FlowField"`; any FlowField or unresolvable field argument ⇒ keep firing at
normal severity with an honest note ("temporary record, but FlowField calculation queries the flow
targets"). FEASIBLE TODAY: `fieldClass` (Normal/FlowField/FlowFilter + `isBlobLike`) is modeled on
BOTH sides (`object-indexer.ts:166-179` native; `symbol-reference-parser.ts:345-349` ABI), and
`RecordOperation.fieldArguments` captures the named fields. Note: the motivating CDO `LoadFiles`
does `Files.CalcFields("File Blob", …)` — Blob fields → in-memory → the downgrade IS correct
there; the policy preserves that while protecting the FlowField case.

### RV-2 — Metamorphic oracle carve-out (MUST) [gemini, follows RV-1]
The oracle property as stated ("adding `temporary` may only remove/downgrade findings") is FALSE
for CalcFields-on-FlowField. Restated property the oracle enforces: adding `temporary` to a
declaration may only remove or downgrade findings, EXCEPT findings on `CalcFields`/
`SetAutoCalcFields` ops whose field arguments include a FlowField — those must be INVARIANT under
the edit. The oracle implementation asserts both directions (suppressed-class shrinks; the
FlowField class is unchanged).

### RV-3 — PD polymorphism CONFIRMED REAL; gemini's "compiler forbids it" claim REFUTED (verified) [empirical]
Compiler probe (alc 18.0.35, runtime 15.0 AND 17.0, real 28.0 symbols): BOTH
`TakesVarPlain(TempRec)` (temp arg → by-var param WITHOUT keyword) AND `TakesVarTemp(PhysRec)`
(physical arg → by-var param WITH `temporary` keyword) compile CLEAN — zero errors, zero warnings.
Components 2/3 are justified; ParameterDependent is a real polymorphism (event subscribers receive
temp records through keyword-less `var Rec` params too — the standard `if Rec.IsTemporary() then
exit;` guard exists for this reason). CONSEQUENCE EXPOSED: the existing L2 rule "by-var WITH
keyword ⇒ Known(true)" (`intraprocedural-refs.ts:135`) is trust-based, not compiler-guaranteed — a
physical instance passed through a temp-keyword param keeps hitting SQL. DECISION: keep
`Known(true)` as the pragmatic industry-contract default (mismatch is a contract violation,
vanishingly rare), DOCUMENT the hazard inline, and note a future detector candidate
("temp-contract violation": substitution can already detect a resolved caller passing
known-physical into a temp-keyword param). Out of this epoch's scope.

### RV-4 — ABI carries the markers; capture is a parser read, not a format gap (verified) [supersedes §1d]
Inspected the real Continia Core 29.0 `SymbolReference.json`: record parameters carry
`TypeDefinition: { Name: "Record", Subtype: {...}, "Temporary": true }` (175 occurrences), and
temp tables carry the property `{"Name":"TableType","Value":"Temporary"}`. The parser DTOs simply
never read them (`AbiParameter` has no temp field; no `TableType` read). §1d is upgraded from
"verify + fallback" to: READ `TypeDefinition.Temporary` (params + return types) and the `TableType`
property (tables) on the ABI path. NOTE the real scope of work [opus-2]: ABI routines currently
have `recordVariables: []` and no per-param tempState (`dependency-projection.ts:101-117`) —
populating them is net-new ABI modeling, budget it (native+ABI shape parity rule).

### RV-5 — FIX the existing last-wins shadowing bug BEFORE tempState backfill (MUST) [opus-2]
`record-types.ts:73-76` builds its name map with unconditional `set` over params→locals→globals —
the GLOBAL overwrites a same-named local/param (last-wins). Today the blast radius is a narrow
tableId fallback; once tempState backfill lands there, a temp GLOBAL shadowing a non-temp LOCAL
would stamp `Known(true)` on the local's ops → silent suppression of a real finding. Flip to
first-wins (`if (!map.has(key))`) FIRST, and order the promotion globals-first/locals-overwrite in
every map (the op-backfill map at `routine-indexer.ts:282`, the binding map at
`intraprocedural-body.ts:188`, and `.find()`-based consumers like `call-resolver.ts:173` need
locals to precede promoted globals). The Rust port has the SAME bug, self-documented
(`alch-engine/src/engine/l3/record_types.rs:8` "LAST-wins") — fix in lockstep.

### RV-6 — Component 3 rewritten: where resolution actually happens (MUST) [opus-1]
The original Component 3 claimed per-path truth with "zero detector changes" — self-contradictory
against the real consumers:
- **d33 needs NO path resolution.** It is intra-routine (`d33:75-82`) and already skips by-var
  params entirely (`paramRecordNames`, `d33:69-86`). The CDO member-var fix is delivered 100% by
  Component 1 (capture → backfill → `op.tempState = Known(true)` → the existing suppression).
- **d1 reads `terminalOp.tempState` / `effect.tempState`** (`d1:86,135,155,398`), never walks
  frames. Per-path truth therefore requires a NEW shared helper — `resolveTempAlongPath(path,
  terminalOp)` in the path-walker module — which steps frame-by-frame toward the path root using
  each hop's `callsiteId` (already on `EvidenceStep`, populated by d1's `buildHopStep`,
  `d1:262-279`) + the callsite's argument binding, applying the same substitution table. d1's
  policy then consumes the resolved state. This IS a (small, shared) detector-side change; the
  spec's claim is downgraded from "zero detector changes" to "no detector POLICY semantics change;
  one shared resolution helper wired into path-walking detectors (d1 first)." The walker's
  `WalkResult` must also expose the callee-param index per hop (net-new walker output) [opus-1].
- **d1 merge-tie rule (new possibility):** `mergeByTerminal` (`d1:174`) merges paths to one
  finding; post-substitution two paths can DISAGREE on temp-derived severity. Rule: the WORST
  severity wins (conservative + deterministic), with the temp note listing both verdicts
  ("temporary via CallerA path; physical via CallerB path").

### RV-7 — Substitution wiring + the parameter-source binding gap (MUST) [opus-1]
- The composition effect loop is EDGE-keyed (`summary-runner.ts:171-188`) while bindings hang off
  `callSites` — thread a `Map<callsiteId, CallArgumentBinding[]>` into `composeRoutineCtx` (the
  c1b pass at `:248-294` shows the pattern; `CombinedEdge.callsiteId` exists per callsite,
  `combined-graph.ts:24-33` — one edge per callsite, so per-callsite substitution is structurally
  free).
- **Binding gap:** `sourceTempState` is captured only for local/recVar sources
  (`intraprocedural-body.ts:238`); an argument that is the CALLER'S OWN PARAM carries
  `sourceParameterIndex` but NO tempState — so a `temporary`-keyword by-var param forwarded onward
  would substitute to PD/Unknown instead of Known(true) (conservative but defeats the forwarding
  case). Fix: at composition, when the binding has `sourceParameterIndex`, resolve through the
  caller's own `recordVariables[param].tempState` (Known(true) keyword param → Known(true);
  keyword-less by-var param → re-symbolize PD(j)).
- **Complete edge-kind table:** `event-dispatch` edges carry no callsiteId (`combined-graph.ts:
  428-439`) → Unknown (verified, matches the open-world claim); `interface`, `object-run`
  (Codeunit/Page/Report.Run), and `dynamic` edges → Unknown explicitly (no binding semantics
  modeled for implicit-Rec-of-run-target in this epoch).
- **Cardinality bound:** dedupe inherited effects by `(operationId, resolvedTempState)` — per op
  the resolved-state space is finite and small ({Known(t), Known(f), Unknown} ∪ PD(callerParams)).
  This bounds the per-routine effect growth that the existing flood guard
  (`summary-runner.ts:189-204`, the documented 100× OOM regression) exists to prevent; the plan
  adds a Base-App-scale perf check before/after.

### RV-8 — Capture completeness, verified against the grammar [opus-2]
- **Protected var sections: NON-ISSUE** — `protected`/`local` is an optional leading child of the
  SAME `var_section` node (`grammar.js:2544-2545`); `extractObjectGlobals`' `var_section` check
  already covers them.
- **Real conservative misses (document, fall to Unknown→fires):** object-level var sections inside
  `#if` (`preproc_conditional_var_block`, grammar.js:2519-2532) and dataitem-scoped `var_section`s
  in Reports/Queries are NOT walked by `extractObjectGlobals`. Acceptable conservative gaps;
  listed, not silently absent.
- **Param typed on a temp table:** the table-level override must RE-RUN at L3 op resolution where
  tableId is known, superseding the `PD(i)` stamped at L2 (a by-var param of a
  `TableType=Temporary` table is `Known(true)` — the precedence rule applies to params too).
- **`RecordVariable.scope`** is a new (optional, to spare the hand-built fixture sites) model field
  on BOTH sides (TS `entities.ts:380` + Rust `PRecordVariable`, `features.rs:220-226`). The
  binding builder's hardcoded `sourceKind: "local"` (`intraprocedural-body.ts:220`) must honor it
  (diagnostic-only mislabel otherwise).
- **`xRec` in `SourceTableTemporary` pages** is `Known(true)` alongside `Rec` [gemini].

### RV-9 — Migration breadth, honestly stated [opus-1, opus-2]
**340 golden files reference `tempState`** across r0/r1a/r2a/r2b/r2c/r2d/r2.5b/r3a*/r4/cli-a/
cli-b/cli-c — member-var ops flipping Unknown→Known move L2, L3-rt, L4, digest, AND finding-level
goldens; substitution additionally moves the R3a-2 per-iteration TRACE-ORACLE baselines for every
PD-touching SCC and the digest tempState merge (`digest-query.ts:546-613`, whose
"physical-if-ANY" asymmetry already encodes the right suppression direction — preserve it). Both
repos. `CACHE_VERSIONS.symbolReader` is currently "17" → "18". The spec's earlier "goldens +
snapshots" phrasing is superseded by this enumeration.

### RV-10 — Claims tempered [opus-1]
- "(temp state uncertain) becomes rare" — NOT for cross-app dependency terminals, whose summaries
  legitimately keep unsubstituted PD (no caller bindings in isolation). Tempered accordingly.
- Fixture additions locked by review: **by-value-of-temp-arg** (the dangerous direction — callee
  `Insert()` on a by-value param is PHYSICAL, must NOT be suppressed; the existing
  `Known(false)` by-value rule is correct and load-bearing, confirmed against AL copy semantics);
  a CalcFields-FlowField-on-temp fixture (must keep firing); a CalcFields-Blob-on-temp fixture
  (must downgrade).

### RV — revised component summary
1. Capture (member vars + TableType + SourceTableTemporary incl. xRec) — unchanged in intent;
   shadowing order + L3 first-wins fix (RV-5) are prerequisites; ABI reads the real markers (RV-4).
2. Substitution — unchanged in semantics; wiring + param-source resolution + edge table +
   cardinality dedup per RV-7.
3. Path-time resolution — exists as a SHARED helper consumed by path-walking detectors (d1 first),
   with the merge-tie rule (RV-6). d33 untouched.
4. RecordRef — unchanged.
5. Detector policy — ONE real policy change after all: the CalcFields/FlowField gate (RV-1) +
   the d1 merge-tie + temp-note wording. Everything else remains automatic via `Known(true)` gates.
6. Oracle — carve-out per RV-2.
