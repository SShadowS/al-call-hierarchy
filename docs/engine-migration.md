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
| **R2 (= R2a+R2b+R2c+R2d)** | Full **source-only L3 resolve** parity: record types + call graph + event graph + coverage at the `resolveModel` (post-resolve / pre-summary) boundary | **COMPLETE** — all four sub-gates SHIPPED; the full source-only L3 surface is at byte-parity with al-sem over the corpus + a native L3-direct oracle per sub-gate |
| **R2.5a** | Symbol-only `.app` reader (ZIP → `NavxManifest.xml` + `SymbolReference.json` → shared `Routine`/`ObjectDecl`/`Table`/`Field` shape) + the **dependency-entity subset of the merged index** at byte-parity (StableObjectId/StableTableId/StableFieldId/StableRoutineId, `signatureFingerprint`, `attributesParsed`, object props, table fields/keys, routine accessModifier, app `sourceKind`), captured POST-`withDependencyArtifacts`/-`resolveModel` with `noDepSummaries:true` — incl. the L3 **extension-field-merge capture-point** (dep `TableExtension` fields merged INTO the base table) | **SHIPPED** — 2/2 fixtures byte-match, `KNOWN_DIVERGENCES` empty, anti-degenerate matrix manifest-oracle-equal (objects=16/tables=2/routines=11; dispatch rb=7/ext=2/bare=6/tbl=1; sourceKind app=1/symbol=1; events pub=1/sub=1; mergedExtFields=1), native structural oracle green (6 checks); ABI-vs-native fix-then-freeze = NO fix needed (native==ABI 1:1). NEXT: **R2.5b** (cross-app L3) |
| **R2.5b** | Cross-app **L3 RESOLUTION** over the merged (workspace + `.app`-dep) index — the R2a–R2d L3 surfaces re-run with deps no longer opaque: (a) a record var typed as a DEP table binds to the dep StableTableId + dep/ws TableExtension fields merge across the boundary; (b) `cu.M()` on a PRESENT dep codeunit resolves to the dep StableRoutineId (the source-only `opaque`→`resolved`/`external-target` transitions; the edge does NOT gate on the dep routine's `internal`/`local` accessModifier); (c) a `[EventSubscriber]` ↔ dep-published `[IntegrationEvent]` links to the dep publisher EventId (both directions); (d) coverage `routinesTotal` counts dep routines + the `unresolvedCallsites` multiset reflects cross-app resolution. Captured POST-`resolveModel` over the merged index (`noDepSummaries:true`), stable ids | **SHIPPED** — all 4 sub-gate differentials byte-match (rt/cg/eg/cov, 1 cross-app fixture each), `KNOWN_DIVERGENCES` empty; cross-app matrices healthy (rt: depBoundRecordVars=2/depBoundRecordOps=2/depExtMergedFields=1; cg: resolvedToDepRoutine=4/memberNotFound=1/opaque=1/externalTarget=1/upgradedBindings=1; eg: resolvedToDepPublisher=1/depSubscriberEdges=1/maybe=1; cov: crossAppResolvedAbsent=4/externalTargetPresent=1/unresolved=2/opaqueApps=2-symbol-only-deps after R3a-0 Fix 2); 4 native oracles per sub-gate green + the cross_app_l3 smoke/poison/aldump tests green. The TWO al-sem latent bugs are now **FIXED in R3a-0** (`opaqueApps` lists the symbol-only deps; the `primaryDependencies`-before-resolve member-opaque branch is live in production) — see the R3a-0 row + the "two latent bugs — FIXED" section below. R2.5 COMPLETE → L3 COMPLETE |
| **R3a-0** | The **semantic oracle epoch** — al-sem fixes the two deferred latent bugs that feed L4 (`81d538a` primaryDependencies-before-resolve + opaqueApps populated; `f1650ba` corpus proof; `93e360d` R2.5b capture stamps `primaryDependencies` BEFORE resolve = production-faithful + the corpus made ALL-FETCHED; artifact schema 1→2, cache `summarySchema` 32→33 / `depCache` 7→8). The Rust R2.5 emitters re-flip to the corrected al-sem; the R2.5 differential must re-pass at ZERO divergences before R3a-1. | **SHIPPED** — BOTH emitters flipped to the FIXED behavior. **Fix 2 (opaqueApps):** the cov emitter passes ALL apps so the `symbol-only` filter lists the dep guids (`["dddddddd-…01","eeeeeeee-…02"]`). **Fix 1 (real ledger):** the cg + cov cross-app emitters thread the REAL declared/fetched ledger (mirroring fixed production+capture al-sem); the Rust fixture `app.json` dropped `Lib Absent` to match the all-fetched corpus. On the all-fetched corpus this is BYTE-INVARIANT (declared=fetched={Lib Core, Lib Ext} → `has_unfetched_declared_dependency` false → `gone.M()` → `external-target` genuinely; object-run → `opaque`). All 4 R2.5b content goldens (rt/cg/cov/eg) byte-match the current al-sem `93e360d`; the cov golden was re-copied. The full R2.5 differential re-passes at 0 divergences, `KNOWN_DIVERGENCES` empty. `tests/r3a0_unfetched_dep_opaque.rs` proves the Fix-1 member-`opaque` resolver branch (unfetched-declared-dep + `cu.M()` → `opaque`, absent from unresolvedCallsites; no-dep control → `external-target`, present). |
| **R3a-1** | The **FIRST L4 sub-gate** — the L4 GRAPH SUBSTRATE: `buildCombinedGraph` (resolved call graph + event graph → `CombinedEdge[]` of kinds direct/method/codeunit-run/report-run/page-run/interface/implicit-trigger/event-dispatch/dynamic + the bipartite event-dispatch edges + the to-less `UncertaintyEdge[]` + the typed `GraphEdge[]`/`typedEdges`, `edgeSortKey`-ordered) + `tarjanScc` (ITERATIVE Tarjan → the reverse-topological SCC condensation; members sorted; `recursive` = size>1 ∨ self-loop), captured POST-`buildCombinedGraph` / POST-`tarjanScc` / PRE-`computeSummaries` (NO summaries, NO dep hooks, source-only) in stable-id form | **SHIPPED** — 158/158 source-only fixtures byte-match, `KNOWN_DIVERGENCES` empty; the anti-degenerate matrix Rust-computed + manifest-oracle-equal (edgesByKind direct=123/method=26/codeunit-run=4/interface=4/implicit-trigger=6/event-dispatch=41; combined=204, uncertainty=93, eventDispatch=41, typed=206, typedEvt=41, sccs=557, recursive=3, multiMember=3); `aldump --r3a1-combined-graph` emits the projection; the native L4-direct structural oracle green (5 invariants: every edge `to` is a real routine id; event-dispatch edges == resolved publisher→subscriber pairs; valid reverse-topological SCC order; `recursive` ⟺ size>1 ∨ self-loop; the SCC partition covers every node exactly once). NEXT: **R3a-2** (the JACOBI fixed-point summary core over this SCC condensation) |
| **R3a-2** | The **SECOND L4 sub-gate** — the **JACOBI FIXED-POINT SUMMARY CORE** (the heart of L4): `computeSummaries`/`runSummaries` composing per-routine `RoutineSummary` CORE bottom-up over the (R3a-1-parity) reverse-topo SCC condensation via a finite monotone JACOBI fixed point (frozen prior-pass snapshot; new-map writes; map swap; `summaryFingerprint` convergence; the `MAX_FIXED_POINT_ITERATIONS=1000` cap). Surface: `dbEffects` (the `effectKeyOf`-keyed merge with via-precedence `direct>implicit-trigger>event-subscriber>dynamic>inherited`), `uncertainties` (caller-side opaque-callee attach + dedup), `parameterRoles` (cross-call entry-req + exit-effect composition via the **branch-aware CFG walker**, incl. resolved `readsFields`/`writesFields` FieldIds), `inRecursiveCycle`, `hasUnresolvedCalls`. Captured POST-`computeSummaries` (NO dep hooks R3a-4, NO cone/coverage R3a-3), source-only, stable-id form | **SHIPPED** — 158/158 source-only fixtures byte-match + the 3 per-recursive-SCC fingerprint-TRACE goldens byte-match (the JACOBI proof: per-iteration fingerprint sequence + iteration count + per-pass `changed` == al-sem ⟹ frozen-snapshot, not Gauss-Seidel), `KNOWN_DIVERGENCES` empty; the anti-degenerate matrix Rust-computed + manifest-oracle-equal + recomputed-from-goldens-equal (routines=567, inheritedEffects=56, viaKinds direct=273/event-subscriber=12/implicit-trigger=3/inherited=55 + dynamic ABSENT, recursiveCycle=13, opaqueCallee=2, crossCallExitEffect=44, uncertaintyKinds unresolved-call=99/dynamic-dispatch=4/external-target=4/interface-open-world=4/ambiguous-overload=4/opaque-callee=2/member-not-found=1/parse-incomplete=1; trace: recursiveSCCs=3, requiring≥2-iter=2, maxIter=4); `aldump --r3a2-summary-core` + `--r3a2-trace` emit the projections; native L4-direct structural oracle green (5 invariants: every inherited effect traces to a callee effect; `effectKeyOf` dedup per routine; via = max over contributing edge-vias; `inRecursiveCycle` ⟺ recursive SCC; uncertainty ⟹ `hasUnresolvedCalls`). The branch-aware CFG walker port (R3a-2 Tasks 1-2 `1e5d4b0`) + the `resolve_field` field-id port (this task) landed. |
| **R3a-3** | The **THIRD L4 sub-gate** — the **CAPABILITY CONE + COVERAGE** (the last two `RoutineSummary` fields): per-routine `capabilityFactsDirect` (the 13 `extractCapabilities` family extractors run over the L3-RESOLVED features — table/commit/dispatch/http/telemetry/isolated-storage/hyperlink/file-blob/background/ui/ui-window-open/events/error — with resolved `resourceId`, the UNREACHABLE-exclusion pass, the full value-source classifier incl. initializer-chasing + member-expression table-field, PLUS the L4 publisher-`publish`-fact injection), `capabilityFactsInherited` (`composeInheritedCones` — the shortest-path-wins bottom-up cone walk over `typedEdges` + the SCC condensation, `inheritedFactKey` dedup, equal-distance tie-breaker, canonical sort), and `coverage` (`CoverageRecord` — directStatus + the monotone inheritedStatus roll-up + the uncertainty-derived reason union from the L4 fixed point — ambiguous-overload/member-not-found/external-target/interface-open-world — + unknownTargets). Captured POST-`computeSummaries` cone pass (NO dep hooks R3a-4), source-only, stable-id form | **SHIPPED** — 159/159 source-only fixtures byte-match (incl. the added `ws-r3a3-equal-distance-tie` tie fixture), `KNOWN_DIVERGENCES` empty; the anti-degenerate matrix Rust-computed + manifest-oracle-equal on the comparable fields + recomputed-from-goldens-equal (routines=573, routinesWithInheritedFacts=142, coveragesWithNonTrivialInheritedStatus=22, provenance direct=556/inherited=246, confidence static=708/unresolved=93/configDynamic=1, op commit=90/delete=6/error-throw=16/execute=15/insert=71/modify=116/publish=95/read=175/send=38/store-write=1/subscribe=122/ui-confirm=1/ui-error=16/ui-message=37/ui-window-open=1/write-blob=2, via self=556/call=192/event-dispatch=54, directStatus complete=560/partial=12/unknown=1, inheritedStatus complete=551/partial=22) PLUS the REAL self-validating BFS counts that supersede the al-sem manifest's conservative proxies (genuine >1-hop shortest witnesses=72, genuine equal-distance ties=15 — both > 0); `extra` is id-free across the corpus so it byte-matches verbatim; `aldump --r3a3-cone-coverage` emits the projection; native L4-direct structural oracle green (provenance/via invariants incl. the event-dispatch no-callsite carve-out; `inheritedFactKey` dedup; every inherited key descends from a direct producer; monotone coverage roll-up well-formedness; the independent-BFS-vs-cone agreement; 16-op / 10-resourceKind family coverage). With R3a-3 SHIPPED, the **FULL source-only L4 `RoutineSummary` is at byte-parity** (core R3a-2 + cone/coverage R3a-3). NEXT: **R3a-4** (the dep producer/consumer hooks — `injectIntraAppCallEdges` feeding the cone cross-app) |
| **R3a-4** | The **FOURTH L4 sub-gate** — the **DEP-ARTIFACT PRODUCER + CONSUMER HOOKS** (the cross-app L4 substrate, the input the R3a-5 cone reads): the embedded-source PRODUCER (`build_dep_artifact_l4` — the engine re-run over a dep `.app`'s embedded `.al` source, projecting `intraAppCallEdges` own→own resolved/deduped/sorted, `citedOperationEvidence` direct-fact witnesses, `depOrderIndex` per-routine order entries + return summaries + a freshness stamp; `summaryMode` "full" only when a body parsed) + the CONSUMER hooks (`inject_intra_app_call_edges` → synthetic direct-call `typedEdges` under the both-ends-in-merged-model guard; `collect_cited_dep_evidence` deduped/sorted; `collect_dep_order_index` under the freshness barrier) + the **stable-id dep-routine PROJECTION** (`DepIdStabilizer`: internal `<modelInstanceId>/<keyHash>[/opN|/csN]` → stable `<appGuid>:<Type>:<Num>#<normalizedSignatureHash>[/opN|/csN]`, cache/modelInstanceId/devFingerprint-INDEPENDENT — NO `dep:<artifactKey>` prefix). Captured POST-`injectIntraAppCallEdges`/`collectCitedDepEvidence`/`collectDepOrderIndex`, stable-id form | **SHIPPED** — 1/1 cross-app fixture (the source-bearing `Dep Chain` dep with the DoIt→DoWrite→Insert chain) **BYTE-MATCHES** the al-sem golden, `KNOWN_DIVERGENCES` empty; the stable-id projection is wired + verified model-instance-independent (`36dce15b…`/`d1b6bb59…` stable routine ids match al-sem exactly); the anti-degenerate matrix (fail-on-zero) Rust-computed + manifest-oracle-equal (intraAppCallEdges=1, injectedTypedEdges=1, citedEvidence=1, orderEntries=2, returnSummaries=2, depOrderIndexPresent=true, freshnessStampFresh=true); `aldump --r3a4-dep-hooks` emits the projection; native L4-direct oracle green (4 invariants: every intraAppCallEdge own→own; injected typedEdge ⟺ an intraAppCallEdge with both ends in the merged model + 1:1 synthetic direct-call; the freshness stamp gates stale artifacts to ABSENT; cited evidence/order entries/return summaries deduped + sorted). The return-summary gate was made faithful to al-sem (`summary === undefined` ≡ "is summarized" — emits a return summary for EVERY own routine incl. bodyless, not gated on `body_available`; observationally equivalent on this corpus, no golden impact). The `depOrderIndex` CONTENTS are count-reduced in the golden projection (per-routine scopeFrame/op/callsite COUNTS, not the full order data) — **R3a-5 owns the full order-index contents** if its cone needs them. NEXT: **R3a-5** (the FULL cross-app L4 summary parity — the cone propagating dep facts to primary callers) |
| **R3a-5** | The **FIFTH + LAST L4 sub-gate** — the **FULL CROSS-APP L4 SUMMARY** (the R3a-1/2/3 L4 path re-run over the merged workspace+dep corpus WITH the R3a-4 dep hooks): the merged index (workspace native routines + EMPTY-feature dep routines carrying a RETAINED summary = the dep's own `via:"direct"` dbEffects + `capabilityFactsDirect`, recovered from the dep's embedded source) feeds `buildCombinedGraph` → `injectIntraAppCallEdges` (the dep intra-app `direct-call` typedEdges) → `computeSummaries` (dep routines as LEAVES; `compute_summaries_with_leaves`) → the cone, so the cone PROPAGATES the dep's `capabilityFactsDirect` through the injected typedEdges + the cross-app resolved member-call typedEdges to PRIMARY callers' `capabilityFactsInherited`, AND the dbEffect compose folds the dep's `via:"direct"` dbEffect into the primary's `dbEffects` as `via:"inherited"`. The FULL `RoutineSummary` (R3a-2 core + R3a-3 cone/coverage + `isDepRoutine`) is projected per routine, STABLE-id form, over the cross-app corpus. Captured POST-`computeSummaries` WITH dep hooks | **SHIPPED** — 1/1 cross-app fixture (the source-bearing `Dep Chain` DoIt→DoWrite→Insert + a symbol-only dep) **BYTE-MATCHES** the al-sem golden (5 summaries: 2 primary + 3 dep), `KNOWN_DIVERGENCES` empty; the dep fact PROPAGATES — primary `UseChain` inherits the dep `DoWrite`'s Insert capabilityFact (`provenance:"inherited"`, witnessCallsiteId on the primary's cross-app callsite, witnessOperationId the dep's own op) AND folds the dep's Insert dbEffect (`via:"inherited"`); the anti-degenerate CROSS-APP matrix (fail-on-zero) Rust-computed + manifest-oracle-equal (primaryRoutinesWithInheritedDepFacts=1, primaryRoutinesWithDepDbEffects=1, coveragesWithOpaqueAppsReason=2, totalCrossAppInheritedFacts=1); `aldump --r3a5-cross-app-summary` emits the projection; native L4-direct oracle green (5 invariants: the cross-app cone fired with provenance=inherited; the witness traces through the injected/cross-app dep edge; coverage reflects the symbol-only-dep opaque surface; the dep routine's own direct fact is unchanged; the dbEffect composes cross-app direct→inherited). The `compute_summaries_with_leaves` seam mirrors al-sem's `isLeaf(r)=r.summary!==undefined` (dep routines are fixed leaves, never recomputed; source-bearing dep routines retain `bodyAvailable=true` so a primary caller is not spuriously flagged `opaque-callee`). With R3a-5 SHIPPED, the **FULL L4 `RoutineSummary` is at byte-parity — source-only (R3a-1/2/3) + cross-app (R3a-5)**. NEXT: **R3b** (Salsa incrementality over the L4 fixed point) |
| **R3a (= R3a-0…R3a-5)** | Full **L4 `RoutineSummary`** parity — the combined-graph + Tarjan SCC substrate, the JACOBI fixed-point summary core, the capability cone + coverage, the dep-artifact producer/consumer hooks, and the FULL cross-app summary with dep-fact propagation — at the post-`computeSummaries` (with dep hooks) boundary | **COMPLETE** — all six sub-gates SHIPPED; the full from-scratch L4 `RoutineSummary` is at byte-parity with al-sem over the source-only AND cross-app corpora + a native L4-direct oracle per sub-gate. NEXT: **R3b** (Salsa incrementality over the L4 fixed point — the novel core: wrap the from-scratch L4 in Salsa queries, prove incremental == from-scratch byte-equal + reverse-cone recompute-minimality) |
| **R3b** | **Salsa INCREMENTALITY over the L4 fixed point** (the novel core, NOT a parity gate — the same L4 OUTPUT, made demand-driven + incremental via a Salsa 0.27 query graph). **Stage 1** wraps the from-scratch L4 in tracked queries (`combined_graph`→`scc_condensation`→interned `SccKey`→the early-cutting projections→`scc_summaries` JACOBI→`routine_summary`/cone) and proves the Salsa-WRAPPED result byte-matches the R3a from-scratch goldens (r3a3 source-only + r3a5 cross-app). **Stage 2** makes the DB persistent + editable (fine-grained per-routine inputs + setters) and proves `incremental == from-scratch` BYTE-equal over 1908 random edits (set-fact / call-edge add-remove / routine add-remove-rename / app-identity / dep-stamp / no-op-at-L4), plus value-equal-carrier no-op early-cutoff + a schedule/DB-provenance/`RUST_HASH_SEED` nondeterminism oracle. **Stage 3** RE-GRANULARIZES `scc_summaries` to depend ONLY on its own SCC's members + edge targets + successor summaries (per-routine `routine_combined_edges`/`routine_uncertainty_edges`/`routine_body_available`/`routine_leaf_summary` queries replace the monolithic `combined_graph` read + full `by_id` scan) and PROVES recompute-MINIMALITY: a localized non-topology edit recomputes only the edited SCC + its reverse cone of callers — a STRICT subset of all SCCs | **SHIPPED** — wrapped-parity (r3a3+r3a5 byte-match from-scratch + golden) + incremental-equality (1908 edits, `KNOWN_DIVERGENCES` empty) + the Stage-3 re-granularization (scc_summaries depends ONLY on its SCC's members+successors; OUTPUT unchanged — all R3a goldens + wrapped-parity + incremental-equality still byte-match) + recompute-MINIMALITY (`tests/r3b_minimality.rs`, the EXIT GATE: by-category WillExecute instrumentation STRUCTURAL/PROJECTION/SUMMARY; the recomputed SUMMARY set ⊆ the reverse cone; STRICT-SUBSET on curated fixtures — a localized leaf-dbEffect edit recomputes only 4/6 SCCs, the 2 unrelated SCCs early-cut, 0 structural; a root-caller edit recomputes exactly 1/6; 111 real multi-SCC fixtures reverse-cone-bounded with 77 witnessing a strict-subset recompute; edge merge/split + add/remove/rename + dep/identity cases stay within the whole-graph cone ceiling AND byte-equal to from-scratch) + the cyclic-fixed-point fingerprint TRACE reproduced THROUGH the Salsa `scc_trace` query (== the R3a-2 per-iteration JACOBI trace == al-sem) on the 3 recursive fixtures. The R3a-5 injection-coverage hardening landed (the `o6_intra_dep_injected_edge_is_load_bearing` native oracle: the MIDDLE dep `DoIt` inherits the inner dep `DoWrite`'s Insert fact PURELY via the injected intra-dep edge — a future injection regression drops it, gating the injection path that the primary-side O1 alone does not). NEXT: **R4** (L5 detectors over the byte-parity + incremental L4 substrate) |
| **R3 (= R3a + R3b)** | Full **L4 summaries — from-scratch (byte-parity) AND incremental (Salsa, reverse-cone-minimal)** | **COMPLETE** — R3a (from-scratch L4 `RoutineSummary` at byte-parity with al-sem, source-only + cross-app) + R3b (the same L4 made demand-driven + genuinely-minimal-incremental via Salsa, with wrapped-parity + incremental-equality + reverse-cone minimality + the fingerprint trace through Salsa all proven). The L4 substrate is now both al-sem-faithful AND incrementally recomputable. NEXT: **R4** (the L5 performance detectors — pure queries over the L4 `RoutineSummary` + cone substrate) |

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

## R2.5a parity status (SHIPPED — `.app` symbol reader + merged-index identity parity) — R2.5's FIRST sub-gate

R2.5a is the FIRST cross-app sub-gate: the **symbol-only `.app` reader** + the
**dependency-entity subset of the merged index** at byte-parity. It ports al-sem's
`.app` ingestion (ZIP header strip → `NavxManifest.xml` + `SymbolReference.json` →
the shared `Routine`/`ObjectDecl`/`Table`/`Field` model shape) and proves the
projected `analysisRole:"dependency"` entities match the TS oracle. It does NOT do
cross-app L3 resolution (that is R2.5b) — events here are `Routine`s with
`kind:"event-publisher"/"event-subscriber"`, NOT a distinct `EventSymbol` entity.

**What R2.5a reproduces.**
- `src/engine/deps/app_package_zip.rs` — `strip_app_header` (≤4096 `PK\x03\x04`
  scan), `normalize_zip_entry_name`, first-match entry pick (TS
  `Object.keys(entries)[0]`), BOM strip.
- `src/engine/deps/app_manifest.rs` — `parse_app_manifest_xml`: `<App>` identity
  (word-boundary-anchored so `CompatibilityId` ≠ `Id`), `<Dependency>` list, and
  `includes_source` (`IncludeSourceInSymbolFile="true"`). The dep app identity used
  for ENTITY ENCODING comes from HERE (the manifest), NOT `SymbolReference.json`'s
  `AppId`.
- `src/engine/deps/symbol_reference.rs` — `parse_symbol_reference`: the
  ROUTINE_BEARING / EXTENSION_ROUTINE_BEARING / BARE / Tables object-array dispatch,
  `classify_abi_arg` (the same `AttributeInfo` shape the native AST path produces),
  `parse_abi_interface_name` (`#<guid>#` strip + unquote), field/key parse.
- `src/engine/deps/projection.rs` — `project_abi_to_index`: `abi_signature_hash`
  (= `sha256Hex(canonical_routine_signature(...))` — the parity crux), access
  modifier from IsInternal/IsLocal, EMPTY features, `bodyAvailable:false`,
  `analysisRole:"dependency"`, the dep object `sourceHash`, field-id resolution.
- `src/engine/deps/merged_index.rs` — the MERGED-INDEX emitter: read the `.app`(s),
  project + merge (append) + **run the extension-field merge** (the capture-point
  invariant, below), then emit the dependency-entity projection in the SAME stable
  JSON shape/key-order as the al-sem goldens.

**The ABI-vs-native fix-then-freeze outcome = NO fix needed.** Task 1's de-risking
vectors (signature / attribute / stable-id-independence) proved native ==
ABI for EVERY component (full `StableObjectId` + `StableRoutineId` + the
`encodeRoutineId` tuple, not just the hash). The two pipelines — native AL source
(L0→L2) and `.app` symbol packages (`parse_symbol_reference` →
`project_abi_to_index`) — converged 1:1 with NO change to the shared
`canonicalRoutineSignature` / attribute classifier; the de-risk gate found no
divergence to freeze.

**The extension-field-merge capture-point invariant.** The al-sem goldens were
captured POST-`resolveModel`, which runs L3's `mergeExtensionFields`: a dep
`TableExtension`'s fields are PHYSICALLY merged INTO the base table (rekeyed to the
base table id / StableFieldId; provenance kept on the extension). So a base table
in the golden carries the extension's field, AND the extension's own table still
retains it under its own id — no double-count. The Rust emitter reproduces that
merge (`merge_extension_fields_projected`, mirroring `extension_fields.rs`); without
it the table goldens diverge. It is the ONLY `resolveModel` step reproduced, because
it is the only one that mutates an IDENTITY-relevant projected field — the
call/event/coverage builders touch graphs/coverage, none of the projected identity
fields, so they are correctly skipped (they belong to R2.5b).

**The fixture-bytes wiring (true byte-parity).** Both sides read the SAME `.app`
bytes: al-sem commits the dep `.app` fixtures (`test/fixtures/r2.5a-deps/<appGuid>.app`,
built deterministically by `buildTestApp`); the al-sem dump COPIES them into each
throwaway workspace rather than rebuilding, and those exact bytes are copied into
the engine at `tests/r2-5a-fixtures/<fixture>/`. (`buildTestApp` was hardened to pin
the ZIP mtime via LOCAL-time Date components so the `.app` bytes are timezone-
independent across machines/processes.)

**Comparison surface (R2.5a).** `aldump --r2.5a-merged-index <app-or-dir>` emits
objects/tables/routines/apps for the dependency entities; the differential
(`tests/r2_5a_differential.rs`) byte-compares against the al-sem goldens over the
FULL projection (not just ids — `attributesParsed`, object props, table fields/keys,
routine accessModifier, app sourceKind all byte-equal). Forbidden L4/summary/cone
keys HARD-FAIL.

**R2.5a's native soundness oracle is structural** (`tests/r2_5a_oracles.rs`, run
against the RUST output — not a transitive byte-match): every dep routine
`bodyAvailable:false` + EMPTY features + `analysisRole:"dependency"`;
`signatureFingerprint == sha256Hex(canonicalRoutineSignature(...))` recomputed
independently; accessModifier matches IsInternal/IsLocal; sourceKind matches
includesSource; the `#<guid>#` interface prefix is stripped; the dep TableExtension
field is merged into the base table (the capture-point invariant, no double-count).

**Result:** 2/2 fixtures byte-match, `KNOWN_DIVERGENCES` empty, the anti-degenerate
matrix healthy + manifest-oracle-equal (objects=16, tables=2, routines=11;
dispatch rb=7/ext=2/bare=6/tbl=1; sourceKind app=1/symbol=1; events pub=1/sub=1/
subArgs=4; mergedExtensionFields=1), all six structural oracle checks green. R2.5a
extends, never replaces, the R1/R2a–R2d goldens.

### Covered vs deferred (R2.5a — honest)

- **Covered:** the symbol-only `.app` reader + the merged-index dependency-entity
  IDENTITY subset at byte-parity (Objects/Tables/Routines/App), the extension-field
  merge capture-point, both `sourceKind` branches, every dispatch class.
- **No-compiler honesty:** the ABI `TypeDefinition.Name` spellings in Task 1's
  vectors are AUTHORED (no `alc` in-env); the vectors prove the two normalization
  PIPELINES converge given matched inputs and enumerate the assumed BC serialization
  as reviewable rows. The `includesSource:true` differential (native embedded `.al`
  vs ABI on the SAME `.app`, under `noDepSummaries:true`) is genuinely two
  independent code paths, not a tautology.
- **Deferred to R2.5b:** ALL cross-app L3 resolution (record-types vs dep tables,
  call graph → dep routines + opaque→resolved/external transitions, event graph →
  dep publishers, coverage `opaqueApps` non-empty); the distinct `EventSymbol`
  entity; the dependency-artifact CACHE serde; dep SUMMARIES (L4/R3).

---

## R2.5b parity status (SHIPPED — cross-app L3 resolution) — R2.5's SECOND + LAST sub-gate — R2.5 COMPLETE → L3 COMPLETE

R2.5b is the SECOND and LAST R2.5 sub-gate: it lifts the SOURCE-ONLY restriction
every L3 sub-gate (R2a–R2d) carried by re-running the FOUR L3 surfaces over
`workspace + deps` — the R2.5a dependency entities are now in the merged index, so
the callsites that resolved to `opaque`/`external-target` source-only now bind to
REAL dep entities. R2.5b is **wiring + cross-app fixtures + anti-degenerate matrices
+ native oracles**, NOT a new L3 algorithm: it FEEDS the R2.5a merged index into the
ALREADY-PORTED `l3_workspace` pipeline (`src/engine/deps/cross_app_l3.rs` wraps the
source-only `l3` projections over the merged model). It extends, never replaces, the
R1/R2a–R2d/R2.5a goldens. Captured POST-`resolveModel` over the merged index
(`noDepSummaries:true`), stable ids; **all four sub-gate differentials byte-match,
`KNOWN_DIVERGENCES` empty.**

**The four cross-app sub-gates (each a `.app`-bearing differential + a native oracle).**

- **R2.5b-a record-types** (`tests/r2_5b_rt_differential.rs` + `r2_5b_rt_oracles.rs`).
  A record var typed as a DEP table binds to the dep `StableTableId`
  (`${depAppGuid}:Table:${number}`) — observable ONLY post-resolve (the dep table
  enters the symbol table via `withDependencyArtifacts`), so it is a genuine
  anti-stale vector (Rev 2 #3). A dep `TableExtension`'s fields merge onto its dep
  base table AND a workspace `TableExtension`'s field merges onto a dep base table —
  the merge crosses the app boundary in BOTH directions. The matrix is fail-on-zero:
  `depBoundRecordVars=2`, `depBoundRecordOps=2`, `depExtensionMergedFields=1`. The
  native oracle asserts the SPECIFIC expected dep `StableTableId` + the exact merged
  `StableFieldId` (NOT "is-a-dep-table"); the corpus carries **≥2 dep tables** so a
  wrong-but-same binding is detectable (Rev 2 #1).
- **R2.5b-b call graph** (`tests/r2_5b_cg_differential.rs` + `r2_5b_cg_oracles.rs`).
  A `cu.M()` on a PRESENT dep codeunit + name/arity match resolves to the dep
  `StableRoutineId` (`resolution:"resolved"`, `to` = the dep routine); a present dep
  object + a missing member → `member-not-found`; an object in an UNfetched declared
  dep → `opaque`; an absent object with all deps fetched → `external-target`. **The
  edge does NOT gate on the dep routine's `internal`/`local` accessModifier** (Rev 2
  #2 — that is an L5/D13 concern): a fixture with an `internal` AND a `local` dep
  callee asserts the edge forms IDENTICALLY regardless (a guard against a
  wrongly-added visibility gate). `argumentBindings` is upgraded in place on the
  resolved edge. Matrix (fail-on-zero): `resolvedToDepRoutine=4`, `memberNotFound=1`,
  `opaque=1`, `externalTarget=1`, `upgradedResolvedBindings=1`. The native oracle
  asserts the four resolved member edges are the four DISTINCT exact dep
  `StableRoutineId`s + the internal/local callees resolve to their exact ids.
- **R2.5b-c event graph** (`tests/r2_5b_eg_differential.rs` + `r2_5b_eg_oracles.rs`).
  A workspace `[EventSubscriber]` to a dep-published `[IntegrationEvent]` links to
  the dep publisher EventId, AND a dep subscriber to a workspace event links (Rev 2
  #7 — the latter rides the parity-projected dep `attributesParsed`). Matrix
  (fail-on-zero): `resolvedToDepPublisher=1`, `depSubscriberEdges=1`, `maybe=1`. The
  native oracle asserts the SPECIFIC expected dep publisher EventId (NOT "links to a
  dep event"); an UNlinked subscriber forms NO false-`resolved` edge — **open-world
  soundness preserved**: cross-app linkage is *target-runs* binding, NOT a refutation
  (per the event-crossed-refutation-unsound-openworld rule).
- **R2.5b-d coverage** (`tests/r2_5b_cov_differential.rs` + `r2_5b_cov_oracles.rs`).
  `routinesTotal` COUNTS dep routines (`coverage.ts:60` `index.routines.length`, no
  `analysisRole` filter); the `unresolvedCallsites` MULTISET reflects cross-app
  resolution — the cross-app member calls that RESOLVED drop OUT, the external-target
  member miss stays IN. Matrix (fail-on-zero): `crossAppResolvedAbsent=4`,
  `externalTargetPresent=1`, `unresolved=2`; **`opaqueApps`=`["dddddddd-…01",
  "eeeeeeee-…02"]` — the two symbol-only deps (R3a-0 Fix 2; the latent bug is FIXED)**.
  The native oracle asserts `opaqueApps` lists the symbol-only deps, the now-resolved
  callsites are ABSENT from the unresolved multiset, the external-target miss is
  PRESENT, and `routinesTotal` counts dep routines.

**Same-bytes cross-app fixtures.** Both sides read the SAME dep `.app` bytes: al-sem
commits them (`test/fixtures/r2.5b-deps/<guid>.app`, built deterministically by
`buildTestApp`); the engine copies those exact bytes into
`tests/r2-5b-fixtures/<fixture>/.alpackages/`. The workspace `.al` files are the
hand-maintained engine mirror of al-sem's INLINE capture-script constants
(`scripts/r2.5b-cross-app-capture.ts`, written to an mkdtemp — al-sem commits no
`.al` workspace dir). The **unified `#[ignore]`d refresh**
(`tests/r2_5b_refresh.rs::refresh_r2_5b_goldens_from_al_sem`, gated on `AL_SEM_DIR`)
re-copies ALL FOUR golden sets + the dep `.app`s from an al-sem checkout in one
command (mirroring the R2.5a refresh) — the documented one-command regen path.

**The L4-leakage POISON guard (Rev 2 #5)** — `tests/cross_app_l3_poison.rs`: a merged
model with BOGUS `summary`/`cone`/`typedEdge`/`intraAppCallEdges` fields present
must produce a BYTE-IDENTICAL L3 projection vs without them (if the output changes,
L3 is reading out-of-scope state). The L3 input boundary is an L3-only merged index;
the four projections HARD-FORBID L4/summary/typedEdges/cone keys on both sides.

### TWO LATENT al-sem BUGS — FIXED in R3a-0 (semantic oracle epoch)

R2.5b originally MIRRORED two latent al-sem bugs (surfaced + source-verified during the
migration). At **R3a-0** (the semantic oracle epoch, al-sem `81d538a`+`f1650ba`) both are
FIXED in al-sem, and the Rust emitters re-flip to the corrected behavior:

1. **`opaqueApps` now lists the symbol-only dep apps** (was structurally `[]`) —
   **FIXED in R3a-0 (Fix 2).** `buildCoverage` (`coverage.ts:37-39`) reads
   `index.identity.apps.filter(sourceKind === "symbol-only")`. al-sem's Fix 2 makes
   `withDependencyArtifacts` stamp the dep `AppIdentity`s (with `sourceKind`, derived
   from the artifact header's `appIdentity.sourceKind`, set at ingest from
   `ref.includesSource`) into `identity.apps` — so the symbol-only dep filter now
   matches. Artifact schema 1→2; cache `depCache` 7→8. The al-sem `r2.5b-cov` golden
   MOVED (`opaqueApps: []` → `["dddddddd-…01","eeeeeeee-…02"]`). **Rust flip:** the cov
   emitter (`project_coverage_cross_app`) now passes ALL apps (workspace + each dep) to
   `build_coverage`, letting the `symbol-only` filter populate `opaqueApps`; the cov
   golden was copied byte-for-byte from al-sem. The cov/smoke/aldump oracles now assert
   the non-empty `opaqueApps`.
2. **`primaryDependencies` stamped BEFORE `resolveModel` → the member-call `opaque`
   branch is live** (was dead) — **FIXED in al-sem R3a-0 (Fix 1, `81d538a` production +
   `93e360d` capture).** al-sem now stamps `identity.primaryDependencies` onto the merged
   index BEFORE `resolveModel` — in BOTH production `analyzeWorkspace` AND the R2.5b
   capture harness (`93e360d`, enforced by a new contract test asserting
   capture===production) — so `hasUnfetchedDeclaredDependency` reads the real declared
   deps DURING resolution: a member miss whose object is absent into an UNFETCHED declared
   dep classifies `opaque` (was over-claiming `external-target`). To keep the cg matrix's
   external-target axis covered under the fixed order, al-sem ALSO made the R2.5b corpus
   **ALL-FETCHED** (removed the prior `Lib Absent` unfetched dep, `93e360d`): with
   declared = fetched = {Lib Core, Lib Ext}, `gone.M()` (absent object, all deps fetched)
   → `external-target` GENUINELY, and `Codeunit.Run("Absent Dep Cu")` (object-run,
   ledger-independent) → `opaque`. **Rust flip:** the cg + cov cross-app emitters
   (`project_call_graph_cross_app` / `project_coverage_cross_app`) thread the REAL
   declared/fetched ledger (was empty); the Rust fixture `app.json` dropped `Lib Absent` to
   match the all-fetched corpus. On the all-fetched corpus this is BYTE-INVARIANT vs the
   empty ledger (the cg golden is byte-unchanged), so all 4 content goldens still match.
   The unfetched-declared-dep member-`opaque` branch is proven out-of-corpus by
   `tests/r3a0_unfetched_dep_opaque.rs` (unfetched-declared-dep + `cu.M()` → `opaque`,
   absent from `unresolvedCallsites`; no-dep control → `external-target`, present). The
   earlier golden-staleness concern (an al-sem capture-harness bug) is **RESOLVED** in
   `93e360d`.

### Covered vs deferred (R2.5b — honest)

- **Covered (cross-app L3 resolution):** record-types bound to dep tables + bidirectional
  extension-field merge; the call graph resolved to dep routines (the
  `opaque`→`resolved`/`external-target` transitions, no accessModifier gate, in-place
  `argumentBindings` upgrade); the event graph linked to dep publishers (both
  directions, open-world target-runs binding); coverage `routinesTotal` counting dep
  routines + the cross-app `unresolvedCallsites` multiset delta. 4/4 sub-gate
  differentials byte-match + 4 native oracles + the smoke/poison/aldump guards.
- **Fixed at R3a-0:** the two latent al-sem bugs above (`opaqueApps` non-empty + the
  member-`opaque` branch) — BOTH Rust emitters re-flipped to the corrected behavior (Fix 2
  applied; Fix 1 real ledger threaded into the cg + cov cross-app projections, byte-invariant
  on the all-fetched corpus). The al-sem capture-order golden-staleness is resolved
  (`93e360d`).
- **Deferred:** the `analysisGaps` per-app gap derivation (tied to the opaqueApps surface,
  revisited at R3/R4); the
  **distinct `EventSymbol` entity as a first-class cross-app entity** (at this capture
  point events project through the same EventSymbol/EventEdge shape as source-only
  R2c — no NEW cross-app entity materializes); the L4 dependency SUMMARIES +
  capability-cone propagation across app boundaries
  (`intraAppCallEdges`/`citedOperationEvidence`/`depOrderIndex`), SCC/fixed-point,
  Salsa incrementality → **R3**; L5 detectors → R4; product wire-in.

### R2.5 COMPLETE → L3 COMPLETE

With R2.5b SHIPPED, **R2.5 (= R2.5a identity + R2.5b cross-app L3) is COMPLETE** — the
symbol-only `.app` reader reproduces the merged-index dependency-entity IDENTITY subset
byte-for-byte (R2.5a) AND the four L3 surfaces RESOLVE over the merged workspace+deps
index at byte-parity (R2.5b). And with the source-only **R2 (R2a–R2d)** + the cross-app
**R2.5 (R2.5a + R2.5b)** both done, **L3 (resolve) is COMPLETE**: record types, call
graph, event graph, and coverage are all at byte-parity with al-sem — source-only AND
cross-app — each with a native L3-direct oracle. NEXT: **R3** (L4 summaries).

---

## R3 (L4 engine / Salsa summaries) — STUB

R3 is the FIRST L4 gate — the genuinely hard INCREMENTAL gate. It lifts parity from
the per-routine / per-edge L3 RESOLUTION surface to the **bottom-up effect summaries**
al-sem composes over the call graph. Scope (al-sem `src/engine/`):

- **Combined graph → Tarjan SCC → condensation.** Build the COMBINED graph (call edges
  + event edges + implicit-trigger edges), run Tarjan's SCC algorithm, and condense it
  to a DAG so recursive cycles collapse to a single fixed-point node.
- **Finite monotone fixed-point `RoutineSummary`.** Compose per-routine effects
  bottom-up over the SCC condensation using a finite monotone lattice so recursive
  cycles CONVERGE (`typedEdges`, `intraAppCallEdges`, `citedOperationEvidence`,
  `depOrderIndex`, the inherited/cone capability facts, the L4-injected `op:"publish"`,
  the L4-augmented coverage status/reasons + per-app `CoverageRecord`). This is where
  every L4-forbidden field R0–R2.5 hard-forbade finally MATERIALIZES.
- **Salsa incrementality — the hard part.** Re-derive only what a source edit
  invalidates (the incremental engine), proving the incremental result EQUALS a clean
  from-scratch recompute (the incremental-parity gate) — beyond the byte-equality the
  R0–R2.5 differentials establish.

R3 reuses the resolved L3 model (R2 + R2.5) verbatim as its input — it ADDS the
summary pass, never replaces the resolution. It extends, never replaces, the
R1/R2a–R2d/R2.5a/R2.5b goldens.

---

## R3a-1 parity status (SHIPPED — combined graph + Tarjan SCC, source-only) — R3's FIRST L4 sub-gate

R3a-1 ports the **L4 GRAPH SUBSTRATE** — the input to the fixed-point summary
(R3a-2). It is the GRAPH + SCC ONLY: no `RoutineSummary` fields, no dep hooks, no
cross-app. Two structures are captured POST-`buildCombinedGraph` / POST-`tarjanScc` /
PRE-`computeSummaries`:

1. **The combined graph** (`buildCombinedGraph`): the sorted `CombinedEdge[]` (kinds
   direct/method/codeunit-run/report-run/page-run/interface/implicit-trigger/
   event-dispatch/dynamic — the call-derived routine→routine edges PLUS the bipartite
   event-dispatch edges from the event graph), the to-less `UncertaintyEdge[]`
   (interface-open-world / dynamic-dispatch / ambiguous-overload / member-not-found /
   external-target / unresolved-call), and the typed `GraphEdge[]` (`typedEdges` — the
   Phase 0b-β capability-cone propagation graph; display SourceAnchors dropped). The
   `edgeSortKey` (`${kind}|${callsiteId ?? operationId ?? eventId ?? ""}|${to}`) order
   is reproduced exactly.
2. **The SCC condensation** (`tarjanScc` — ITERATIVE Tarjan, no recursion): the SCC list
   in REVERSE-TOPOLOGICAL order (callees before callers), each `{members (sorted),
   recursive}` where `recursive` = size>1 ∨ a self-loop. The reverse-topo ORDER is part
   of the comparison surface (it is the bottom-up composition order R3a-2 consumes).

**Capture / parity surface.** `assemble_and_resolve_workspace_default(ws)
.project_r3a1_combined_graph()` (the source-only `indexWorkspace → resolveModel →
buildCombinedGraph → tarjanScc → projectR3a1` mirror) projects all ids to stable form
(StableRoutineId / StableObjectId / StableTableId / StableEventId / stable callsiteId /
operationId). `aldump --r3a1-combined-graph <ws>` emits the same projection on the CLI.

**Differential (`tests/r3a1_differential.rs`).** For each of the 158 committed
source-only goldens (`tests/r3a1-goldens/*.r3a1.golden.json` — copied from al-sem; the
`.app`-bearing + empty fail-closed fixtures the al-sem dump EXCLUDED never enter the
corpus), the Rust projection is structurally positional-diffed against the golden. **All
158 byte-match; `KNOWN_DIVERGENCES` empty.** A recursive forbidden-key scan hard-fails on
any later-gate field (dbEffects / uncertainties / parameterRoles / coverage / summary /
capabilityFacts* / the dep-hook outputs). The `#[ignore]`d `AL_SEM_DIR`-gated
`refresh_goldens_from_al_sem` (in `tests/differential.rs`) regenerates the goldens from
an al-sem checkout (step b7).

**Anti-degenerate matrix (fail-on-zero).** Computed from the RUST output (so it proves
the graph + SCC actually FIRE, not "empty == empty"): ≥3 distinct combined-edge kinds, ≥1
uncertainty edge, ≥1 event-dispatch edge, nonzero typedEdges, ≥1 recursive SCC, ≥1
multi-member SCC. The corpus totals are `edgesByKind {direct=123, method=26,
codeunit-run=4, interface=4, implicit-trigger=6, event-dispatch=41}`, combined=204,
uncertainty=93, eventDispatch=41, typed=206, typedEvt=41, sccs=557, recursive=3,
multiMember=3 — equal to BOTH the al-sem `manifest.json` matrix block AND the matrix
recomputed independently from the goldens.

**Native L4-direct structural oracle (`tests/r3a1_oracles.rs`).** Run over the Rust
output (NOT a transitive byte-match), 5 invariants: (1) every `CombinedEdge.to`/`.from`
is a real routine id in the model; (2) the event-dispatch edges == the event graph's
resolved publisher-routine → subscriber-routine pairs (by eventId); (3) `tarjanScc` is a
VALID reverse-topological order (every edge `from→to` has `scc[to] ≤ scc[from]`, with
`scc[to] == scc[from]` only inside a recursive SCC); (4) `recursive` ⟺ (size>1 ∨
self-loop); (5) the SCC partition covers every node exactly once (no node missing, none
in two SCCs; every member a real routine id). All green.

With R3a-1 SHIPPED, the L4 graph substrate is at byte-parity.

---

## R3a-2 parity status (SHIPPED — JACOBI fixed-point summary core, source-only) — R3's SECOND L4 sub-gate

R3a-2 ports the **heart of L4** — the finite monotone **JACOBI fixed point** that composes
per-routine `RoutineSummary` CORE bottom-up over the (R3a-1-parity) reverse-topo SCC
condensation. Capture point: POST-`computeSummaries`, source-only, stable-id form. The
comparison surface is the summary CORE ONLY — `dbEffects`, `uncertainties`,
`parameterRoles`, `inRecursiveCycle`, `hasUnresolvedCalls`; the cone/coverage (R3a-3),
`fieldEffects` (lazy/detector), and the dep-hook output (R3a-4) are HARD-FORBIDDEN (never
declared on the projected types + scanned for on both sides).

**The JACOBI discipline (the load-bearing correctness rule).** Each pass FREEZES the entire
prior-pass summary map; ALL reads within a pass see the frozen snapshot; writes go to a NEW
map; the maps swap at end of pass. This is JACOBI, not Gauss-Seidel. Both reach the same
fixed point (monotone lattice) but via DIFFERENT trajectories — a different iteration count
and a different per-pass `changed` sequence. The per-recursive-SCC fingerprint TRACE oracle
captures the per-iteration fingerprint sequence + iteration count + `changed`, so a
trajectory divergence is caught HERE (before R3b's Salsa makes it incremental).

What's proven:

1. **Full-corpus summary-core differential** (`tests/r3a2_differential.rs`): 158/158
   source-only fixtures byte-match (`aldump --r3a2-summary-core` ≡ `project_r3a2`), positional
   structural diff over the canonically-sorted projection; `KNOWN_DIVERGENCES` empty.
2. **Fingerprint-TRACE differential** (`tests/r3a2_trace_differential.rs`): the 3
   recursive-SCC-bearing fixtures' per-iteration fingerprint sequences + iteration counts +
   per-pass `changed` flags byte-match the al-sem trace goldens — the JACOBI proof.
3. **Anti-degenerate matrix** (Rust-computed, manifest-oracle-equal + recomputed-from-goldens-equal):
   routines=567, inheritedEffects=56, viaKinds {direct=273, event-subscriber=12,
   implicit-trigger=3, inherited=55} (the 4 reachable kinds present; `dynamic` ABSENT in the
   source-only corpus), recursiveCycle=13, opaqueCallee=2, crossCallExitEffect=44,
   uncertaintyKinds {unresolved-call=99, dynamic-dispatch=4, external-target=4,
   interface-open-world=4, ambiguous-overload=4, opaque-callee=2, member-not-found=1,
   parse-incomplete=1}; trace stats recursiveSCCs=3, requiring≥2-iter=2, maxIter=4.
4. **Native L4-direct structural oracle** (`tests/r3a2_oracles.rs`, 5 invariants over the RUST
   output): every inherited effect (`via != "direct"`) traces to a combined-graph callee
   carrying the same effectKey; `effectKeyOf` dedup holds per routine; the merged `via` equals
   the MAX over the contributing edge-vias (`via_for_edge_kind(edge.kind)`); `inRecursiveCycle`
   ⟺ the routine's R3a-1 SCC is recursive; any routine with ≥1 uncertainty has
   `hasUnresolvedCalls = true`.

The branch-aware CFG walker (the path-aware entry-req + exit-effect facts, if/case/loop
branch-join) + the JACOBI loop were ported in Tasks 1-2 (`1e5d4b0`); the `resolve_field`
field-id resolution for `readsFields`/`writesFields` was ported in Task 3 (see the
readsFields/writesFields note in the summary above — it is IN R3a-2, not deferred).

With R3a-2 SHIPPED, the per-routine summary core is at byte-parity and the JACOBI trajectory
is proven over the corpus.

---

## R3a-3 parity status (SHIPPED — capability cone + coverage, source-only) — R3's THIRD L4 sub-gate — **FULL source-only L4 RoutineSummary COMPLETE**

R3a-3 ports the **last two `RoutineSummary` fields**, completing the source-only L4 summary:

1. **`capabilityFactsDirect`** — all **13** `extractCapabilities` family extractors run over
   the L3-RESOLVED routine features (so `resourceId` is the resolved StableTableId/EventId/
   ObjectId, not the L2-stripped R1d form), in al-sem's orchestrator order: table → commit →
   dispatch → http → telemetry → isolated-storage → hyperlink → file-blob → background → ui →
   ui-window-open → events(subscribe) → error. Plus the L4 **publisher-`publish`-fact
   injection** (one `publish` fact per `EventSymbol` whose `publisherRoutineId` is the
   routine, `eventClass` mapped via al-sem's `mapEventKindToClass` default-Integration rule).
   The **unreachable-exclusion** pass drops every op/callsite/record-op whose
   `controlContext === "unreachable"` before family dispatch (the L3 assembly now applies the
   R1b control-context lattice so the flag is present). The **value-source classifier** is the
   full port: literal/enum/database-ref/parameter/member-expression(table-field)/constant-var
   with one-hop initializer-chasing (capped at depth 3), reading the L2-captured
   `VariableSymbol.initializer` now forwarded onto `L3Variable`.
2. **`capabilityFactsInherited`** — `composeInheritedCones` (ported in R3a-3 Task 2): the
   shortest-path-wins bottom-up cone walk over `typedEdges` + the SCC condensation, the
   `inheritedFactKey` dedup, the equal-distance tie-breaker (`repKey`/`edgeSortKey`), the
   canonical sort.
3. **`coverage`** (`CoverageRecord`) — `directStatus` + the monotone `inheritedStatus`
   roll-up + the reason union. The reasons fold in the **uncertainty-derived** coverage
   reasons from the L4 fixed point (summary-runner.ts:565-596): a routine carrying an
   `ambiguous-overload` / `member-not-found` / `external-target` / `interface-open-world`
   uncertainty has its `directStatus` downgraded complete→partial and the reason forwarded
   through the coverage cone (which also adds the routine to `unknownTargets`). Source-only
   `interfaceImplsKnowledgePartial` is false, so the coarser `interface-impls-unknown-in-deps`
   add-on never co-fires.

Captured POST-`computeSummaries` cone pass; NO dep hooks (R3a-4). The `aldump
--r3a3-cone-coverage` emitter projects the cone+coverage in the al-sem golden shape.

The differential (`tests/r3a3_differential.rs`) byte-matches **159/159** source-only
fixtures (incl. the added `ws-r3a3-equal-distance-tie` two-equal-paths fixture),
`KNOWN_DIVERGENCES` empty. The anti-degenerate matrix is Rust-computed, manifest-oracle-equal
on the comparable fields AND recomputed-from-goldens-equal — and, per the Task-1 review note,
computes the **REAL self-validating BFS counts** (genuine >1-hop shortest witnesses = 72,
genuine equal-distance ties = 15) that supersede the al-sem manifest's conservative proxies
(`factsWithMoreThan1HopWitness` = every inherited fact; `equalDistanceTies` = the tie
fixture's routines). The `extra` field is **id-free across the entire 159-fixture corpus**
(all `extra` value-sources are literal / constant-var-with-unknown-initializer; the single
table-field carries `tableId: "unknown"`), so it byte-matches verbatim — no stable-projection
of `extra` is needed on either side.

The native L4-direct structural oracle (`tests/r3a3_oracles.rs`) is green over every fixture:
direct facts carry `provenance=direct`/`via=self`; inherited facts carry
`provenance=inherited`/`via!=self` + a first-hop `witnessCallsiteId` (with the legitimate
`event-dispatch`/`implicit-trigger` no-callsite carve-out the goldens confirm); the
`inheritedFactKey` dedup holds per routine; every inherited key descends from a direct
producer (the cone invents nothing); the coverage roll-up is monotone + well-formed
(`complete` ⟹ no reasons / no unknownTargets; non-`complete` ⟹ ≥1 reason or ≥1 unknownTarget;
sorted/deduped); an INDEPENDENT BFS oracle agrees with the cone on which routines carry
inherited facts and the genuine >1-hop count never exceeds the total inherited facts; and the
16-op / 10-resourceKind family coverage gate proves the ported families actually fire.

With R3a-3 SHIPPED, the **FULL source-only L4 `RoutineSummary` is at byte-parity** — the CORE
(R3a-2: dbEffects / uncertainties / parameterRoles / inRecursiveCycle / hasUnresolvedCalls) +
the cone/coverage (R3a-3: capabilityFactsDirect / capabilityFactsInherited / coverage).
**NEXT: R3a-4** — the dep producer/consumer hooks (`injectIntraAppCallEdges` feeding the cone
cross-app + the dep-artifact producer projection), the cross-app capability cone.

---

## R3a-4 parity status (SHIPPED — dep-artifact producer + consumer hooks + stable-id projection) — R3's FOURTH L4 sub-gate — the cross-app L4 SUBSTRATE

R3a-4 ports the **dependency-artifact L4 producer + the consumer hooks** — the cross-app
substrate the R3a-5 cone reads. It is the bridge that makes a dep `.app`'s behavior
visible to the primary app's L4 summaries:

1. **The embedded-source PRODUCER** (`build_dep_artifact_l4`, `src/engine/deps/dep_artifact_l4.rs`)
   — the engine RE-RUN over a dep `.app`'s embedded `.al` source (the isolated dep L3
   model, `analysisRole "dependency"`, `sourceUnitId = dep:<appGuid>:<relpath>`), then a
   compact projection: `intraAppCallEdges` (own→own resolved direct/method/interface
   edges, deduped by `(from,to)`, sorted), `citedOperationEvidence` (the direct-capability
   witnesses — `r.Insert` at its file:line anchor with `controlContext`), and
   `depOrderIndex` (per-routine order entries + return summaries + a freshness stamp).
   `summaryMode` is `"full"` only when ≥1 body parsed (a symbol-only / parse-failed dep →
   the order index is ABSENT, a barrier).
2. **The CONSUMER hooks** — `inject_intra_app_call_edges` (each intra-app edge with BOTH
   ends in the merged model → one synthetic `direct-call` `typedEdge`, `syntaxKind`
   "synthetic"); `collect_cited_dep_evidence` (deduped by operationId, sorted);
   `collect_dep_order_index` (collected only from artifacts whose stamp is FRESH — the
   freshness barrier; stale/absent/schema-mismatch → skipped).
3. **The stable-id dep-routine PROJECTION** (`DepIdStabilizer`,
   `src/engine/deps/r3a4_projection.rs`) — THE key fix. A dep routine's INTERNAL id
   (`<modelInstanceId>/<keyHash>[/opN|/csN]`) is modelInstanceId/devFingerprint-keyed →
   NOT reproducible by another engine. The projection maps it to the STABLE
   `<appGuid>:<Type>:<Num>#<normalizedSignatureHash>[/opN|/csN]` form
   (appGuid/signature-derived → cache/modelInstanceId/devFingerprint-INDEPENDENT, NO
   `dep:<artifactKey>` prefix). The Rust dep routine already carries `stable_routine_id`
   = `to_stable_object_id(object_id) + "#" + normalized_signature_hash` — exactly al-sem's
   stabilizer base; the `/opN`/`/csN` suffix (everything after the two-`/`-part routine
   id) is re-attached. The emitted ids (`36dce15b…`/`d1b6bb59…`) match al-sem's golden
   EXACTLY and are verified model-instance-independent.

**Captured POST-`injectIntraAppCallEdges`/`collectCitedDepEvidence`/`collectDepOrderIndex`**,
stable-id form. The cross-app fixture is the source-bearing `Dep Chain` codeunit (50300)
with the intra-dep call chain `DoIt() → DoWrite() → r.Insert(true)` — non-hollow, so every
payload surface is exercised.

**Result — the EXIT GATE:** the 1 cross-app fixture **BYTE-MATCHES** the al-sem golden
(`tests/r3a4-goldens/cross-app-dep-hooks.r3a4.golden.json`), `KNOWN_DIVERGENCES` empty. The
anti-degenerate matrix (fail-on-zero) is Rust-computed + al-sem-manifest-oracle-equal:
intraAppCallEdges=1, injectedTypedEdges=1, citedEvidence=1, orderEntries=2,
returnSummaries=2, depOrderIndexPresent=true, freshnessStampFresh=true. `aldump
--r3a4-dep-hooks <workspace>` emits the projection. The native L4-direct oracle is green
(4 invariants: every intraAppCallEdge own→own; injected typedEdge ⟺ an intraAppCallEdge
with both ends in the merged model + 1:1 synthetic direct-call; the freshness stamp gates
stale artifacts to ABSENT; cited evidence / order entries / return summaries deduped +
sorted). The R3a-4 Task-2 vector parity (7 tests) stays green.

**Return-summary gate made faithful:** the Task-2 review found the Rust gated the
per-routine return summary on `!body_available`, while al-sem gates on
`summary === undefined` (faithful = "is the routine summarized" — which after
`runSummaries` is true for ALL own routines, incl. bodyless ones, which get an
unknown/partial summary). The Rust gate now matches al-sem's logic (emit a return summary
for every own routine; only the ORDER ENTRY is gated on having scope frames + ops/callsites).
On this corpus there are no bodyless own routines (native interface methods are not
routines), so the gates are observationally equivalent — no golden impact, but the Rust is
now faithful for a future corpus with a bodyless own dep routine.

**SCOPE NOTE — order-index contents:** the R3a-4 golden projects the `depOrderIndex` /
`depRoutineOrderEntries` as COUNTS (per-routine `scopeFrameCount` / `operationOrderCount` /
`callsiteOrderCount`, + the index `routineCount` / `returnSummaryCount`), NOT the full
order data. **R3a-5 owns the full order-index CONTENTS** if its cross-app cone needs the
per-op/-callsite ordering to sequence dep effects.

With R3a-4 SHIPPED, the **cross-app L4 substrate is at byte-parity** — the dep producer
payloads + the injected typed edges + the collected evidence/order data are all
byte-identical to al-sem, with the dep-routine ids in the stable, engine-reproducible form.
**NEXT: R3a-5** — the FULL cross-app L4 summary: inject the R3a-4 dep `intraAppCallEdges`
into `typedEdges` BEFORE the cone walk so `composeInheritedCones` propagates a dep's
`capabilityFactsDirect` to its primary-app callers.

---

## R3a-5 parity status (SHIPPED — full cross-app L4 RoutineSummary + dep-fact propagation) — R3's FIFTH + LAST L4 sub-gate — **R3a COMPLETE**

R3a-5 re-runs the R3a-1/2/3 L4 path over the MERGED workspace+dep corpus WITH the R3a-4
dep hooks, so the cone PROPAGATES a dep routine's capability facts to its primary-app
callers. It is the final R3a sub-gate; with it, **R3a is COMPLETE** — the full from-scratch
L4 `RoutineSummary` is at byte-parity, source-only (R3a-1/2/3) AND cross-app (R3a-5).

**What R3a-5 wires (the propagation path).** `project_r3a5_cross_app`
(`src/engine/l4/capability_cone.rs`) mirrors al-sem's `analyzeWorkspace` order EXACTLY:

1. **Merged cross-app L3** (`build_cross_app_l3_from_workspace`): the workspace native
   routines + the EMPTY-feature dep routines (the symbol-reference projection, like
   al-sem's `EMPTY_FEATURES`). The cross-app member calls resolve (`UseChain.DoIt()` /
   `.DoWrite()` → the dep StableRoutineIds; `UseSymbolOnly.DoSomething()` → the symbol-only
   dep). The merged dep-routine internal ids EQUAL the R3a-4 dep-artifact ids (both content+
   modelInstanceId-derived) — so the injected edges line up.
2. **Recovered RETAINED dep facts** (`recover_dep_retained`): re-run each source-bearing
   dep `.app`'s embedded-source assemble+resolve (the R3a-4 producer path) to recover, per
   dep routine, its RETAINED summary (`base_intraprocedural_summary` → the dep's own
   `via:"direct"` dbEffects), its RETAINED `capabilityFactsDirect`, and its direct coverage
   — exactly the fields al-sem keeps on the dep artifact (`dependency-pipeline.ts:632`:
   `dbEffects.filter(via==="direct")` + `capabilityFactsDirect`). Source-bearing dep
   routines also get `bodyAvailable` restored to `true` (al-sem spreads the embedded-source
   routine), so a primary caller is NOT spuriously flagged `opaque-callee`.
3. **`buildCombinedGraph` + `compute_summaries_with_leaves`**: dep routines are FIXED LEAVES
   carrying their retained summary (al-sem's `isLeaf(r)=r.summary!==undefined`); they are
   pre-seeded into the final map and NEVER recomputed, and primary callers FOLD them via the
   cross-app combined edges — `UseChain → DoWrite` folds the dep's Insert dbEffect as
   `via:"inherited"`. (The new `compute_summaries_with_leaves` is the seam;
   `compute_summaries` delegates with an empty leaf map, so R3a-1/2/3 are untouched.)
4. **`injectIntraAppCallEdges` → the cone**: the dep intra-app `direct-call` edges
   (`DoIt → DoWrite`) are appended to the typed-edge graph, then `composeInheritedCones`
   walks it. The dep's `capabilityFactsDirect` (seeded from the retained facts, since the
   merged dep features are EMPTY) propagate through the injected edges (intra-dep) AND
   through the cross-app resolved member-call typed edges to the PRIMARY caller's
   `capabilityFactsInherited`.

**The propagation FIRED.** The primary `UseChain`
(`33333333-…:Codeunit:71000#cd29682…`) inherits the dep `DoWrite`'s
(`cccccccc-…:Codeunit:50300#d1b6bb59…`) Insert capabilityFact —
`provenance:"inherited"`, `via:"call"`, `witnessCallsiteId` on UseChain's own cross-app
callsite (`…71000#cd29…/cs1`), `witnessOperationId` the dep's own Insert op
(`…50300#d1b6…/op0`) — AND folds the dep's Insert dbEffect into its own `dbEffects` as
`via:"inherited"` (operationId the dep's op). The dep `DoIt` inherits the same fact via the
INJECTED `DoIt → DoWrite` edge. The symbol-only `DoSomething` is opaque → `UseSymbolOnly`
coverage is `partial` with an `opaque-dependency` reason + the dep routine in
`unknownTargets`.

**The result.** 1/1 cross-app fixture (5 summaries: 2 primary + 3 dep) **BYTE-MATCHES** the
al-sem golden (`cross-app-full-summary.r3a5.golden.json`), `KNOWN_DIVERGENCES` empty. The
anti-degenerate CROSS-APP matrix (fail-on-zero — the source-only "no propagation" green is a
FAILURE here) is Rust-computed + manifest-oracle-equal:
`primaryRoutinesWithInheritedDepFacts=1`, `primaryRoutinesWithDepDbEffects=1`,
`coveragesWithOpaqueAppsReason=2`, `totalCrossAppInheritedFacts=1`.
`aldump --r3a5-cross-app-summary` emits the projection. The native L4-direct oracle
(`tests/r3a5_oracles.rs`, vs the RUST output) is green (5 invariants: the cone fired with
provenance=inherited; the witness traces through the cross-app/injected dep edge; coverage
reflects the symbol-only-dep opaque surface; the dep routine's own direct fact is unchanged;
the dbEffect composes cross-app direct→inherited). The R3a-1/2/3/4 + R2.5b suites stay green.

**With R3a-5 SHIPPED, R3a (= R3a-0 … R3a-5) is COMPLETE** — the full from-scratch L4
`RoutineSummary` is at byte-parity with al-sem over the source-only AND cross-app corpora,
each sub-gate with a native L4-direct oracle.

---

## R3b status (SHIPPED — Salsa incrementality over the L4 fixed point) — R3 COMPLETE

R3b makes the from-scratch R3a L4 fixed point INCREMENTAL via a Salsa 0.27 demand-driven
query graph — WITHOUT changing the OUTPUT (the same byte-parity L4 `RoutineSummary`, only
made recomputable on edits). It shipped in three stages:

- **Stage 1 (wrapped parity).** The from-scratch L4 is wrapped in tracked queries:
  `combined_graph` → `scc_condensation` (the structural Tarjan pass) → an interned `SccKey`
  (= the sorted-member-`StableRoutineId` set; a merge/split mints a NEW key) → the
  early-cutting projections (`scc_members`/`scc_successors`/`scc_is_recursive`/
  `scc_for_routine`) → `scc_summaries(scc_key)` (the internal R3a JACOBI over the SCC's
  members, depending on its successor `scc_summaries`) → `routine_summary` + the cone
  (`inherited_facts`/`coverage`). The Salsa-WRAPPED projection byte-matches the R3a
  from-scratch goldens (r3a3 source-only + r3a5 cross-app). `tests/r3b_wrapped_parity.rs`.

- **Stage 2 (incremental equality).** The DB is made persistent + editable (fine-grained
  per-routine inputs + a setter per editable field). `incremental == from-scratch` proven
  BYTE-equal over **1908 random edits** (set-fact / call-edge add-remove / routine
  add-remove-rename / app-identity / dep-stamp / no-op-at-L4), with value-equal-carrier
  no-op early-cutoff and a schedule/DB-provenance/`RUST_HASH_SEED` nondeterminism oracle.
  `tests/r3b_incremental_equality.rs` + `tests/r3b_incremental_nondeterminism.rs`.

- **Stage 3 (recompute-MINIMALITY — the EXIT GATE).** The summary query is
  RE-GRANULARIZED: `scc_summaries(scc_key)` no longer reads the monolithic `combined_graph`
  + a full per-routine scan. Instead it builds a PER-SCC mini combined graph from its
  MEMBERS' own per-routine queries (`routine_combined_edges` / `routine_uncertainty_edges`),
  member-only base/routine/leaf maps, and per-target `routine_body_available` /
  `routine_leaf_summary` reads — so it depends ONLY on {its members' inputs + edges} ∪
  {its edge targets' bodyAvailable + retained leaf summaries} ∪ {successor `scc_summaries`}
  ∪ {`scc_members`/`scc_successors`/`scc_is_recursive`(this key)}. An edit isolated to an
  unrelated SCC leaves all of those value-equal ⇒ the query BACKDATES. `tests/r3b_minimality.rs`
  (the EXIT GATE) proves recompute-minimality with by-category `WillExecute` instrumentation
  (STRUCTURAL = `combined_graph`/`scc_condensation`, accounted separately as they may
  recompute broadly on a topology edit; PROJECTION = the early-cutting per-SCC/per-routine
  queries; SUMMARY = `scc_summaries`/`scc_trace`/`routine_summary`/`inherited_facts`/
  `coverage`/`cones`, the BOUNDED set). The recomputed SUMMARY set ⊆ the reverse dependency
  cone of the changed inputs; on curated fixtures a localized leaf-dbEffect edit recomputes
  only **4 of 6 SCCs** (the 2 unrelated SCCs early-cut, 0 structural), a root-caller edit
  recomputes exactly **1 of 6**, and across **111 real multi-SCC corpus fixtures** the bound
  holds with **77 witnessing a strict-subset recompute**. Topology (edge merge/split),
  churn (routine add/remove/rename), and dep/identity edits stay within the whole-graph cone
  ceiling AND byte-equal to from-scratch.

The cyclic fixed point is reproduced THROUGH Salsa: a new `scc_trace(scc_key)` tracked query
runs the SAME re-granularized per-SCC computation with the R3a `collect_trace` hook, so a
recursive SCC's incremental recompute reproduces the EXACT R3a-2 per-iteration JACOBI
fingerprint trace (== al-sem) on the 3 recursive fixtures. The R3a-5 injection-coverage
hardening landed as the native `o6_intra_dep_injected_edge_is_load_bearing` oracle: the
MIDDLE dep `DoIt` inherits the inner dep `DoWrite`'s Insert fact PURELY via the injected
intra-dep edge (it has no direct Insert of its own), so a future injection regression drops
the fact and the oracle fails — gating the injection path that the primary-side O1 (the
primary also calls `DoWrite` directly) does not. `KNOWN_DIVERGENCES` empty.

**With R3b SHIPPED, R3 (= R3a + R3b) is COMPLETE** — the L4 summaries are at byte-parity
from-scratch (source-only + cross-app) AND incrementally recomputable with reverse-cone
minimality. **NEXT: R4** — the L5 performance detectors, pure queries over the byte-parity +
incremental L4 `RoutineSummary` + cone substrate.

---

## R4-0 status (SHIPPED — the L5 shared substrate + harness + first detector)

R4-0 stands up the L5 detector layer in `src/engine/l5/` (greenfield) and proves the full
**substrate → detector → stable-projection → fingerprint → differential** path at byte-parity.

- **al-sem side (Task 1, pushed):** `scripts/r4-finding-projection.ts` (the stable `Finding[]`
  projection — the comparison surface), `scripts/dump-r4-findings.ts`, the 7-fixture smoke
  goldens (`scripts/r4-goldens/`), the contract test. Two-stage opus review folded the
  convergent MUST-FIXes (stable map over ALL routines incl. dep; re-sort in stable space so the
  order is reproducible from stable ids alone — both byte-invariant on the single-app smoke).
- **engine Task 2a (the query substrate):** `reverse_call_graph` / `entry_points` /
  `capability_query` / `transaction_spans` + `FullRoutineSummary` (re-unifies the cone's
  direct/inherited facts + coverage that the Rust core `RoutineSummary` keeps separate). 25
  native ground-truth-free oracles. Role threaded as `dep_routine_ids: &BTreeSet` (empty ⇒
  all-primary source-only); `access_modifier` + `internalReachableExternally` are explicit
  inputs (model-wiring deferred to the D14/R4-G wave).
- **engine Task 2b (the harness + d4):** the `Finding` shape + the stable projection (serde
  field order mirrors al-sem's projection INSERTION order, not the TS interface decl;
  single-pass longest-match id-replacement; re-sort in stable space) + `fingerprint_of` (over
  INTERNAL ids, plain UTF-8 sha256) + `to_confidence` (full uncertainty→cappedBy map) +
  `run_detectors` (catch_unwind→diagnostic, role-scope filter, `(detector via compareNatural,
  primaryLocationKey, rootCauseKey)` sort) + `detector_context` (eager indexes + per-routine
  `FullRoutineSummary` via the `compose_cone_over_graph` seam) + `path_walker` (bounded DFS
  20/500 WITH uncertainty accumulation) + d4 + `aldump --r4-findings` + `tests/r4_differential.rs`.
  Additive L2→L3 forward (`L3Routine.loops`, `L3RecordOperation.{loop_stack,field_argument_infos}`)
  — non-breaking (the L3 projections are field-allowlisted; the full R0–R3 suite stays green).
- **Result:** `ws-d4-repeated-get` byte-matches its al-sem golden end-to-end (4437 bytes, fp
  `1613cafbb8edc2bf`); the other 6 smoke fixtures run clean to the L5 boundary and flip on as
  their wave's detector lands; `KNOWN_DIVERGENCES` empty.
- **Tracked cross-cutting follow-up (NOT bundled):** al-sem `compareStrings` is UTF-16 code-unit
  order while Rust `str::cmp` is UTF-8 byte order — they differ only for non-BMP chars, the same
  ASCII/BMP corpus assumption R0–R3's `cmpStable → str::cmp` port already rests on. A true fix is
  a whole-engine comparator change, tracked separately.

**NEXT: R4-A** (the intraprocedural detectors — d5/d10/d11/d18/d19/d20/d21/d29/d36, pure
`routine.features` reads), then B…G per the plan.

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
| L3 (deps) | **R2.5a** (`.app` symbol reader + merged-index identity parity) | **DONE** — 2/2 fixtures byte-match + anti-degenerate matrix + manifest oracle + 6 structural oracle checks; extension-field-merge capture-point reproduced; ABI-vs-native fix-then-freeze = NO fix needed (pipelines converged 1:1) |
| L3 (deps) | **R2.5b** (cross-app L3 resolution) | **DONE** — 4/4 sub-gate differentials byte-match (rt/cg/eg/cov) + 4 native oracles + smoke/poison/aldump guards; cross-app matrices healthy; `KNOWN_DIVERGENCES` empty; the TWO al-sem latent bugs (`opaqueApps`; member-opaque) **FIXED in R3a-0** — both Rust emitters re-flipped to the corrected behavior |
| L3 (deps) | **R2.5 (= R2.5a + R2.5b)** | **CROSS-APP L3 COMPLETE** — `.app` identity parity + cross-app L3 resolution at byte-parity |
| **L3 (resolve)** | **L3 = R2 (source-only) + R2.5 (cross-app)** | **L3 COMPLETE** — record types + call graph + event graph + coverage at byte-parity, source-only AND cross-app, each with a native L3-direct oracle |
| L4 (graph) | **R3a-1** (combined graph + Tarjan SCC, source-only) | **DONE** — 158/158 source-only fixtures byte-match + the anti-degenerate matrix (≥3 edge kinds, ≥1 uncertainty, ≥1 event-dispatch, nonzero typed, ≥1 recursive + ≥1 multi-member SCC) Rust-computed + manifest-oracle-equal + recomputed-from-goldens-equal; `aldump --r3a1-combined-graph` emitter; native L4-direct structural oracle green (5 invariants); `KNOWN_DIVERGENCES` empty |
| L4 (summaries) | **R3a-2** (the JACOBI fixed-point `RoutineSummary` core over the SCC condensation, source-only) | **DONE** — 158/158 source-only fixtures byte-match + the 3 per-recursive-SCC fingerprint-TRACE goldens byte-match (the JACOBI proof) + the anti-degenerate matrix (≥1 inherited effect, 4 reachable via-kinds + dynamic absent, ≥1 opaque-callee, ≥1 cross-call exit-effect, ≥1 recursive-cycle routine; ≥1 recursive SCC requiring ≥2 iterations) Rust-computed + manifest-oracle-equal + recomputed-from-goldens-equal; `aldump --r3a2-summary-core` + `--r3a2-trace` emitters; native L4-direct structural oracle green (5 invariants); `KNOWN_DIVERGENCES` empty |
| L4 (summaries) | **R3a-3** (the capability cone + coverage over the SCC condensation, source-only) | **DONE** — 159/159 source-only fixtures byte-match (incl. the `ws-r3a3-equal-distance-tie` fixture) + the anti-degenerate matrix (≥1 routine with inherited facts; REAL BFS genuine >1-hop witnesses=72 + genuine equal-distance ties=15; ≥1 non-trivial inheritedStatus; each provenance/confidence/via kind; 16-op family coverage) Rust-computed + manifest-oracle-equal-on-comparable + recomputed-from-goldens-equal; `aldump --r3a3-cone-coverage` emitter; native L4-direct structural oracle green (6 oracle tests); `KNOWN_DIVERGENCES` empty; `extra` id-free → byte-matches verbatim |
| L4 (summaries) | **R3a (= R3a-1 + R3a-2 + R3a-3)** | **SOURCE-ONLY L4 RoutineSummary COMPLETE** — graph substrate + JACOBI summary core + capability cone/coverage at byte-parity |
| L4 (deps) | **R3a-4** (dep-artifact producer + consumer hooks + stable-id dep-routine projection) | **DONE** — 1/1 cross-app fixture BYTE-MATCHES the al-sem golden, `KNOWN_DIVERGENCES` empty; the stable-id dep-routine projection (`DepIdStabilizer` → `appGuid:Type:Num#sigHash`, NO `dep:` prefix) wired + model-instance-independent + al-sem-equal; anti-degenerate matrix (fail-on-zero, intraAppCallEdges=1/injected=1/cited=1/orderEntries=2/returnSummaries=2/depOrderIndexPresent/freshnessStampFresh) manifest-oracle-equal; `aldump --r3a4-dep-hooks` emitter; native L4-direct oracle (4 invariants); return-summary gate made al-sem-faithful (`summary===undefined` ≡ summarized, not `body_available`). The `depOrderIndex` order-index CONTENTS are count-reduced in the projection — R3a-5 owns the full contents if its cone needs them |
| L4 (summaries) | **R3a-5** (the FULL cross-app L4 summary — the cone propagating dep facts to primary callers) | **DONE** — 1/1 cross-app fixture BYTE-MATCHES the al-sem golden (5 summaries: 2 primary + 3 dep), `KNOWN_DIVERGENCES` empty; the dep fact PROPAGATES (primary `UseChain` inherits the dep `DoWrite`'s Insert capabilityFact `provenance:"inherited"` + folds its dbEffect `via:"inherited"`); the anti-degenerate CROSS-APP matrix (fail-on-zero: primaryWithInheritedDepFacts=1/primaryWithDepDbEffects=1/coveragesWithOpaqueAppsReason=2/totalCrossAppInheritedFacts=1) manifest-oracle-equal; `aldump --r3a5-cross-app-summary` emitter; native L4-direct oracle (5 invariants); the `compute_summaries_with_leaves` seam mirrors al-sem's `isLeaf` (dep routines as fixed leaves) |
| L4 (summaries) | **R3a (= R3a-0 … R3a-5)** | **L4 RoutineSummary COMPLETE** — graph substrate + JACOBI summary core + capability cone/coverage + dep producer/consumer hooks + FULL cross-app summary with dep-fact propagation, at byte-parity source-only AND cross-app |
| L4 (Salsa) | **R3b** (Salsa incrementality over the L4 fixed point — the novel core) | **DONE** — wrapped-parity (r3a3+r3a5 byte-match from-scratch + golden) + incremental-equality (1908 edits byte-equal) + Stage-3 re-granularization (`scc_summaries` depends ONLY on its SCC's members+successors via per-routine `routine_combined_edges`/`routine_uncertainty_edges`/`routine_body_available`/`routine_leaf_summary`; OUTPUT unchanged) + recompute-MINIMALITY (`tests/r3b_minimality.rs` EXIT GATE: by-category WillExecute instrumentation; SUMMARY set ⊆ reverse cone; STRICT-SUBSET on curated fixtures — localized edit recomputes 4/6 SCCs, 2 unrelated early-cut; root-caller 1/6; 111 real fixtures reverse-cone-bounded, 77 strict-subset witnesses; topology/churn/dep cases cone-bounded + byte-equal) + the cyclic fixed-point fingerprint TRACE reproduced THROUGH the Salsa `scc_trace` query (== R3a-2 == al-sem) + the R3a-5 injection-coverage hardening oracle (O6); `KNOWN_DIVERGENCES` empty |
| **L4** | **R3 (= R3a + R3b)** | **L4 COMPLETE** — summaries from-scratch (byte-parity, source-only + cross-app) AND incremental (Salsa, reverse-cone-minimal). **NEXT: R4** (L5 detectors over the L4 substrate) |
| L5 (detectors) | **R4-0** (the shared detector SUBSTRATE + harness + the first detector + the differential) | **DONE** — `src/engine/l5/` greenfield: query substrate (reverse-graph/entry-points/capability-query/transaction-spans, 25 native oracles, Task 2a) + harness (Finding + stable projection + fingerprint + confidence + registry/run_detectors + detector_context + path_walker, Task 2b) + d4 ported + `aldump --r4-findings` + `tests/r4_differential.rs`. **ws-d4-repeated-get byte-matches its al-sem golden end-to-end** (4437 bytes, fp `1613cafbb8edc2bf`); full suite green; `KNOWN_DIVERGENCES` empty. The other 6 smoke fixtures run clean to the L5 boundary, flipped on per wave |
| L5 (detectors) | **R4-A** (intraprocedural — d4/d5/d10/d11/d18/d19/d20/d21/d29/d36) | **DONE** — all 10 byte-match (positive goldens + per-detector neutrals); additive L2→L3 forwards (loops/loop_stack/field_argument_infos/identifier_references/unreachable_statements/routine source_anchor) non-breaking; the d29 modify-event regex reduced to a proven-equivalent literal matcher |
| L5 (detectors) | **R4-B** (metadata — d22/d33) | **DONE** — both byte-match; shared `unquoted_field_name` extracted; `tableById` + `L3Field.field_class` reads |
| L5 (detectors) | **R4-C** (event/call-graph — d7/d12/d38 source-only; **d13/d16 DEFERRED**) | **PARTIAL** — d7 (inline Tarjan SCC over the combined graph)/d12/d38 byte-match; `DetectorContext.event_graph` + `parse_routine_attributes` (obsolete/internalProc) added. **d13 (cross-app internal call) + d16 (obsolete-routine call) DEFERRED** — their positives need a dynamically-built dep `.app` (al-sem tests use `buildTestApp`); they land with the **cross-app pipeline entry** (folded into R4-D) |
| L5 (detectors) | **R4-D** (capability-query) | **MOSTLY DONE (11/14)** — d8/d9/d34/d35 (txn-span/cone), d32 (+access_modifier forward), d1/d2/d48 (path-walker, on the PW-0 substrate: op-classification/table-display/path-merge/actionable-anchor + the walk-evidence uncertainty wiring), d43/d44/d45 (event-flow, on the `event_flow.rs` substrate incl. `collect_relay_subscribers`). **DEFERRED (3): d50** (needs the absent `rootClassifications`/RootKind + `L3Object.inherent_commit_behavior` substrate), **d13/d16/d17** (cross-app — needs the cross-app L5 pipeline entry: a DetectorContext from `build_r3a5_cross_app_base` + `dep_routine_ids` role threading + al-sem building/committing the `buildTestApp` dep `.app`) |
| L5 (detectors) | **R4-E** (record-flow / parameterRoles) | **DONE** — d3/d37/d39/d40/d41/d42 (parameterRoles + deriveLoadStates + the cross-call transitive trio); added `ctx.parameter_roles_by_routine` + `ctx.upgraded_bindings_by_callsite` (proven 1:1 positional join) + the `L3RecordVariable.temp_state` forward |
| L5 (detectors) | **R4-G** (dead-routine / commit-in-lifecycle) | **DONE** — d14 (reachable-roots BFS; wired `find_reachable_roots` over `access_modifier`) + d46 (commit-in-lifecycle; `L3Object.object_subtype` forward) |
| L5 (detectors) | **R4-F** (ordering-facts — THE HARDEST) | **NOT STARTED** — the gate for d47[smoke]/d49/d51 is porting the **`engine/ordering-facts.ts` pass on the `src/snapshot/` + `src/digest/` subsystem (~10,252 LOC / 40 files: `composeSnapshot` → `digestQuery` → `computeOrdering`/ordering-engine → the WRITE_PENDING_AT_EXTERNAL_IO / EXTERNAL_IO_BEFORE_COMMIT / WRITE_PENDING_AT_UI / EXTERNAL_IO_IN_EVENT_SUBSCRIBER_TXN / IO_BEFORE_ESCAPING_ERROR labels)** — by far the largest single substrate port in R4; warrants its own spec→plan→staged-implementation sub-phase. Then d47/d49/d51 |
| L5 (detectors) | **R4 exit gate** | **PENDING** — after R4-F + the deferred d50/d13/d16/d17: all 42 detectors byte-match + the engine ANALYSIS-COMPLETE (L0–L5 byte-parity, L4 incremental) |
| product | — | not started |

With R2.5b shipped, **R0 + R1 + R2 (R2a–R2d) + R2.5 (R2.5a + R2.5b) are done — L3 is
COMPLETE**: the source-only L3 surface is at byte-parity, the symbol-only `.app` reader
reproduces the merged-index dependency-entity IDENTITY subset byte-for-byte, and the
four L3 surfaces RESOLVE over the merged workspace+deps index at byte-parity (record
types → dep tables, call graph → dep routines, event graph → dep publishers, coverage
counting dep routines). The TWO latent al-sem bugs (`opaqueApps` always `[]`; the
member-call `opaque` branch dead because `primaryDependencies` was stamped after
`resolveModel`) are **FIXED in R3a-0** (the semantic oracle epoch): `opaqueApps` now
lists the symbol-only deps (Rust cov emitter re-flipped, golden re-copied), and the cg +
cov cross-app emitters thread the REAL declared/fetched ledger mirroring fixed production
al-sem — with the member-`opaque` resolver branch proven by
`tests/r3a0_unfetched_dep_opaque.rs`. The corpus is ALL-FETCHED (al-sem `93e360d` removed
the prior `Lib Absent` unfetched dep + made the capture stamp `primaryDependencies` before
resolve), so the flip is byte-invariant and all 4 content goldens still match.

**R3a-1 (the FIRST L4 sub-gate) is now SHIPPED** — the L4 GRAPH SUBSTRATE
(`buildCombinedGraph` + `tarjanScc`) is at byte-parity with al-sem over the full
158-fixture source-only corpus, captured POST-`buildCombinedGraph` / POST-`tarjanScc` /
PRE-`computeSummaries` (no summaries, no dep hooks). The Rust-computed anti-degenerate
matrix equals BOTH the al-sem `manifest.json` block AND the matrix recomputed from the
goldens (combined=204 across ≥3 edge kinds, uncertainty=93, eventDispatch=41, typed=206,
sccs=557 incl. 3 recursive + 3 multi-member), and the native L4-direct structural oracle
green (every edge target a real routine id; event-dispatch edges == resolved
publisher→subscriber pairs; valid reverse-topological SCC order; `recursive` ⟺ size>1 ∨
self-loop; the SCC partition covers every node exactly once).

**R3a-2 (the SECOND L4 sub-gate — the heart of L4) is now SHIPPED** — the JACOBI
FIXED-POINT SUMMARY CORE (`computeSummaries`/`runSummaries`) is at byte-parity with al-sem
over the full 158-fixture source-only corpus, captured POST-`computeSummaries` (no dep
hooks, no cone/coverage). The Rust-computed anti-degenerate matrix equals BOTH the al-sem
`manifest.json` block AND the matrix recomputed from the goldens (routines=567,
inheritedEffects=56, viaKinds direct=273/event-subscriber=12/implicit-trigger=3/inherited=55
with `dynamic` ABSENT in the source-only corpus, recursiveCycle=13, opaqueCallee=2,
crossCallExitEffect=44), AND the 3 per-recursive-SCC fingerprint-TRACE goldens byte-match
(recursiveSCCs=3, ≥2-iteration=2, maxIter=4) — proving the Rust fixed point is **JACOBI**
(frozen prior-pass snapshot), not Gauss-Seidel: a Gauss-Seidel trajectory would reach the
same final summary but diverge on the per-iteration fingerprint sequence / iteration count.
The native L4-direct structural oracle is green (every inherited effect traces to a callee
effect; `effectKeyOf` dedup per routine; via = max over contributing edge-vias;
`inRecursiveCycle` ⟺ recursive SCC; uncertainty ⟹ `hasUnresolvedCalls`).

The R3a-2 port landed across two efforts: the **branch-aware CFG walker** + the JACOBI loop
(Tasks 1-2, `1e5d4b0`), and the **`resolve_field` field-id port** (this task — see the
readsFields/writesFields note below).

**readsFields/writesFields — field-id resolution is IN R3a-2 (not deferred).** Task 2 had
left `resolve_field` stubbed to `None`, assuming the al-sem goldens carried empty
`readsFields`/`writesFields`. The full-corpus differential surfaced this as the ONLY
divergence: al-sem POPULATES these on 10 fixtures (e.g. `ws-paramfx`,
`ws-event-read-after-write`, `ws-overload-field-discriminator`) with resolved StableFieldIds.
The fix was small and faithful — al-sem's `resolveField` is a case-insensitive field-name
lookup in the parameter's resolved table — so this task ported it: a workspace `FieldIndex`
(`(tableId, lowercased field name) → internal FieldId`, built over the L3 tables with
extension fields already merged) threaded through `compute_summaries` →
`base_intraprocedural_summary` → `compute_record_roles` → `resolve_field`, plus a fix to
`stable_field_id` to mirror al-sem's `toStableFieldId` (split on the LAST `/`, not a literal
`/field/` segment). NO al-sem projection change was needed. After the port the full corpus
byte-matches with `KNOWN_DIVERGENCES` empty.

**NEXT:** the capability cone (R3a-3), the coverage roll-up (R3a-3), the dep hooks (R3a-4),
full cross-app (R3a-5), Salsa incrementality, R4 (L5 detectors), and the product surface.
