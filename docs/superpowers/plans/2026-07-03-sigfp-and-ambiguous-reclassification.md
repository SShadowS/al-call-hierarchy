# Source sig_fp identity + AmbiguousResolved reclassification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

> Status: **v2.1** (round 2 = both GO-WITH-CHANGES → closing items folded; the addenda sections are BINDING and supersede
> ALL conflicting text). **Round-2 closers:** (1) a mixed/collapsed candidate set must never CONSTRUCT
> `DispatchShape::AmbiguousOverload` at all — prevalidate every candidate concrete-exact-non-collapsed BEFORE choosing the
> shape; mixed → the existing Unknown path (fixture asserts NOT `Complete`, NOT `dispatch_shape:"ambiguous_overload"`).
> (2) STALE-TEXT SCRUB (binding replacements): the Interface `push→extend` at the T4 key-facts + Files lines is DROPPED
> (no interface extend; interface/table populations stay Unknown this plan, NO escape hatch); T2 Step-1(d) reads "normal
> overloads get distinct ids unmarked; same-key duplicates collapse unmarked; same-id/different-key survivors are
> collision-guard-marked/fail-closed"; T1 Step-1(b) is SKIP-ONLY with a counted diagnostic; T3's classify test = "all
> concrete exact non-collapsed AmbiguousDispatch routes under direct same-object AmbiguousOverload"; the `.dependencies`
> audit is an explicit PREFLIGHT TASK 0 before T1 (T5 only documents/closes). (3) EXPORT PROJECTION: `graphify_export`
> must not leak dead-code semantics to BC-Brain — for `Condition::AmbiguousDispatch` routes emit an explicit may-fire
> signal (e.g. `condition:"ambiguous_dispatch"` documented as may-fire + a `may_fire:true` field or equivalent additive
> key); define the DTO mapping in T3 and pin it with an export fixture. (4) COLLISION-GUARD OBSERVABILITY: count
> guard-aborted Unknown degradations separately in the T5 report; if the guard fires beyond the 8 known CDO pairs'
> signatures, treat as a threshold alert (investigate, don't mask). Eighth resolution arc (master `9b798fb`, CDO primary real-`unknown` 0.83% /
> `unknown=151`: CompoundReceiver 51, OverloadAmbiguous 56, UntrackedReceiver 18, MemberNotFound 25 (tier split 12/13),
> BuiltinPrecedenceCollision 1; `genuine_wrong=0`). Two grounding reports (this session, file:line + CDO-measured) fixed
> the design AND the sequencing:
> 1. **Source `sig_fp=0` aliasing** (opus-elevated; the untreated twin of the fixed ABI collapse): two same-name/same-arity
>    param-type-differing SOURCE overloads alias ONE `RoutineNodeId` (**8 measured CDO pairs** — a real AL idiom). Dispatch
>    already fails closed (aliased pairs correctly count as 2 candidates → decline). The REAL gaps: (d) publisher
>    witness/span corruption (`emit_event_flow_edges` + `BodyMap` last-write-wins — a marked publisher's EventFlow edge can
>    carry its SIBLING's span, inviting (from,site) dedup to drop a fan-out) and (e) merged caller identity (two overloads'
>    outgoing edges indistinguishable under one node).
> 2. **The `OverloadAmbiguous=56` reclassification** (data-backed: uniformly genuine >1-visible-same-arity): candidates
>    carried as non-default-reachable routes under a NEW taxonomy trio — never reuse `ManualBinding` (a factual runtime
>    claim) or `Polymorphic` (open-world completeness + interface-applicability teeth would misfire — both code-confirmed).
> 3. **SEQUENCING (grounded):** the reclassification NEEDS the sig_fp identity fix first — a candidate set deduped on
>    aliased `RoutineNodeId`s would collapse a genuine 2-overload ambiguity into a false-appears-resolved (the exact
>    footgun `index.rs:157-168`'s own comment warns about). So: T1 marker (fast net) → T2 real identity → T3 taxonomy
>    mechanics → T4 candidate-carrying + the metric-definition change → T5 audit + close.
> 4. **The 13 workspace-tier MemberNotFound stay DEFERRED** (honest: needs genuinely new proven-absent machinery; the
>    preprocessor union-read actually favors absence proofs but needs its own confirming fixture — recorded for the next
>    plan). The `.dependencies` special-casing audit (user-requested) rides as T5's first step.

**Goal:** Give source overloads real identity (fixing publisher-span + caller-attribution fidelity), then honestly
reclassify the 56 genuine compile-time ambiguities out of real-`unknown` as candidate-carrying `AmbiguousResolved` —
an EXPLICIT, documented metric-definition change — with zero false `Source`/`Catalog` (`genuine_wrong` stays 0).

**Tech Stack:** Rust (edition 2024). No new dependency. No `engine::l3`/`engine::l2` import in `src/program/resolve`
(grep-guarded). FOREGROUND cargo ALWAYS (no background runs/monitors). `CDO_WS="U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud"`,
SINGLE tests.

## Key facts (verified on `9b798fb` — the two grounding reports are the authority; key anchors below)

**sig_fp (T1/T2):**
- `sig_fp: 0` hardcoded for source at **5 independent reconstruction sites**: `node_extract.rs:452` (extract_nodes),
  `body_map.rs:65`, `full.rs:210` + `:573`, `stub.rs:68` — a manual-parity contract (stub.rs:30-34 admits it). Any real
  sig_fp MUST be preceded by centralizing these into ONE shared constructor, or ids silently diverge (worse than today).
- `RoutineNode.param_sig_key` is already populated for source (`node_extract.rs:248`, lowercased `|`-joined param type
  texts; `""` for ABI). `RoutineDecl.Param.ty: Option<String>` carries verbatim type text (`decl.rs:199`).
- Dedup (`build.rs:348`) does NOT collapse same-id different-`param_sig_key` source entries — both survive under one id;
  NO marker exists for source (unlike `abi_overload_collapsed`).
- SAFE today (verified): `resolve_in_object` candidate sets hold 2 physical entries for an aliased pair (pre_filter_count=2
  → declines; fixture `ws_overload_collision...` pins it); `routine_node_for_type_query` only reaches proven-unique ids.
  UNSAFE today: `emit_event_flow_edges` (resolver.rs:2059, `body_map.get_with_path` at :2109 — last-write-wins span) when
  BOTH siblings are publishers; caller-identity merge (build.rs:316 "deferred overload-dispatch" doc).
- Blast radius of real sig_fp (grounded): semantic goldens SAFE (`GoldenSiteKey`/`CanonicalKey` never encode sig_fp —
  differential.rs:58-83, semantic_golden.rs:112-132); graphify `al:rtn:` ids ephemeral (no committed fixture matches);
  ~100 `#[cfg(test)]` `sig_fp: 0` literals mostly self-consistent (audit the few running real parsed AL); the fnv1a
  length-delimited primitive from `abi_ingest::param_type_fp` is the fingerprint to reuse (no subtype/degradation fields
  needed — source type text is fully qualified).

**Reclassification (T3/T4):**
- NEW trio: `Condition::AmbiguousDispatch` ("exactly one fires at runtime, chosen by argument-type dispatch this engine
  cannot perform; not user-conditional") + `DispatchShape::AmbiguousOverload` (mapped in `completeness_for_shape`
  (full.rs:288-301) to `SetCompleteness::Complete` — the candidate set is CLOSED, unlike Polymorphic's
  Partial{ReverseDependentImplementers}) + `ObligationOutcome::AmbiguousResolved` (a new explicit `classify_obligation`
  branch (edge.rs:464-500) distinguishing all-AmbiguousDispatch from all-ManualBinding).
- Consumer sweep (compiler-forced where matches are exhaustive): `Histogram` new field (edge.rs:533-598) + the DOCUMENTED
  duplicate `count_into_histogram` (full.rs:825-862); `graphify_export::project_edge` new arm (one GEdge per candidate
  route — the existing per-route loop handles it; `obligation:"ambiguous_resolved"`, `dispatch_shape:"ambiguous_overload"`,
  `condition:"ambiguous_dispatch"` — all additive, no renames/reorders; BC-Brain consumes); `dispatch_shape_str`/
  `condition_str` arms; **aldump's hand-built JSON (aldump.rs:1307-1336) is NOT compiler-forced — a silently-missable
  manual touch, name it in the task**; `route_applicability` falls through `_ => {}` (semantic_golden.rs:2001) with the
  new shape — verify green, don't assume; `witness_contract_holds` is per-route, unaffected by cardinality.
- Candidate-carrying: `resolve_in_object` → `Option<(DispatchShape, Vec<Route>)>` (the file's own convention —
  `member_catalog_route` etc. resolver.rs:1416-1458); 7 call sites mechanical; **the two Interface fan-out sites
  (:1793-1811, :1833-1846) need `push`→`extend` — an implementer's own overload set can fan out inside an
  already-Polymorphic edge (a real nesting wrinkle: design-review it, don't silently absorb)**. Per-candidate routes via
  `make_routine_route` (Source for source tiers, Opaque+AbiSymbol for SymbolOnly — never bare Abi), each carrying
  `Condition::AmbiguousDispatch`; the collapse-marker guard applies per-candidate (a marked candidate contributes an
  `AbiCollapsedOverload`-flavored Unresolved route, not a false Opaque/Source — explicit decision).
- **Scope caveat (grounded):** not all 56 necessarily emit from `resolve_in_object`'s `_` arm — `resolve_bare`'s
  table-scope `TableScopeOutcome::Ambiguous` (resolver.rs:1102-1104, CROSS-OBJECT ambiguity — different candidate
  objects) and the interface per-implementer `matching != 1` arm (resolver.rs:1848-1852, conflates arity-0 with >1)
  also emit the reason. T4 Step 0 partitions the 56 by emission site on CDO FIRST; only the same-object
  `resolve_in_object` population reclassifies this plan (the others get their own honest treatment or stay Unknown —
  decide from the measured partition, documented).
- **Metric-definition change (MANDATORY docs):** the reclassified edges leave `unknown`/`real_unknown_rate`/
  `unknownByReason` automatically (all derive from `classify_obligation`). The charter's "real-`unknown` = routes with
  `Evidence::Unknown`" (charter §5 :229 + §8 :265-268), `ObligationOutcome`'s doc (edge.rs:446-461), and
  `real_unknown_rate`'s doc (edge.rs:502-504) get an explicit addendum recording the change + measured before/after;
  ratchets re-derived (never loosened); CHANGELOG states it plainly.
- **Audit-gate interaction (grounded hypothesis — VERIFY on CDO):** fresh currently projects `{}` targets for these sites;
  if L3's golden is also empty → they land in `fresh_extra` (UNGATED — grep-confirmed no ceiling). If any site's L3
  target is non-empty, it's structurally ⊆ fresh's candidate set → `fresh_ahead_dispatch`, but increments the ZERO-MARGIN
  `FRESH_WRONG_CEILING=149` — adjudicate + re-derive with a dated note. `genuine_wrong` must stay 0.

## Round-1 review addenda (BINDING — supersedes conflicting text elsewhere)

**T4 — the inverse cardinal sin (both reviewers, Critical):** all-candidates-`fires_by_default=false` must NOT make the
edge a traversal dead end (exactly ONE candidate WILL fire at runtime). Keep per-candidate `false` for must/default
traversal + best-evidence scoring, BUT: add an explicit may-reachability rule — change-impact/may traversal includes ALL
routes of an `AmbiguousResolved` edge (a `may_reachable_routes()` accessor or the documented consumer rule); AUDIT every
`default_reachable_routes()` consumer and switch may-semantics consumers; a regression test proving change-impact sees
BOTH candidates.

**T4 — strict `AmbiguousResolved` preconditions (Critical):** classify as `AmbiguousResolved` ONLY when: shape is direct
same-object `AmbiguousOverload`; the candidate set is closed + non-empty; EVERY route carries `Condition::AmbiguousDispatch`;
NO route has `Evidence::Unknown`; NO candidate is collapse-marked (ABI or source-alias); every candidate is a concrete
exact route (`Source`, or exact `Opaque`+`AbiSymbol`). A mixed/degraded set STAYS `Unknown` (with the collapse reason).
Classifier unit test + a T4 mixed-set fixture.

**T4 — interface nesting OUT OF SCOPE (Critical):** do NOT extend implementer overload fan-outs into a Polymorphic edge
(flattening corrupts both semantics: Complete-vs-Partial and the per-implementer grouping). ONLY the direct same-object
`resolve_in_object` population reclassifies; the interface per-implementer `matching!=1` and table-scope Ambiguous
populations stay `Unknown` this plan. The `push→extend` idea is DROPPED; the nested case gets a fixture asserting it does
NOT become `AmbiguousResolved`/`Complete`.

**T2 — sig_fp normalization + the marker's true post-T2 role (Critical):** normalize ONLY lexer-insensitive differences
(case, trim, safe whitespace) — never strip quotes/resolve ID-vs-Name synonyms without compiler backing (under-normalization
→ duplicate-split = tolerable noise since a real object declares each overload once; over-normalization → residual alias =
the cardinal risk). Include every compiler overload-identity component (verify `var`, array rank, subtype qualifiers —
document what AL overload identity actually comprises). The T1 marker's post-T2 role is a **same-id/different-normalized-key
COLLISION GUARD** (fail closed on any residual alias) — NOT "fires only for true duplicates" (true duplicates collapse
unmarked; a post-T2 same-id/different-key survivor = a collision → decline). Rename/reframe accordingly.

**T2 — 5-site audit BEFORE the constructor (Important):** classify each of the 5 sites (concrete identity materialization
vs lookup/probe key vs stub semantics) and verify `RoutineDecl.Param.ty` texts are genuinely IN SCOPE at each — a site
that would silently default to empty params would produce divergent ids (worse than today). The parity test proves all
CONCRETE-identity paths agree for the same decl.

**T1 — SKIP, don't synthesize (Important):** the event-publisher guard SKIPS a dual-publisher-aliased span (with a counted
diagnostic), never emits a synthetic zero-span (dedup-collision + fake-witness risks). T1 Step-4 expectation: resolution
stats byte-identical; event/trigger digests move ONLY if a dual-publisher alias exists on CDO (record). T2 restores precise
spans via real identity; the guard remains post-T2 as the collision-guard net.

**T4 — pre-declared fresh/L3 acceptance matrix (Important):** BEFORE Step 0: (1) L3-empty + all candidates source-visible
→ `fresh_extra`, acceptable; (2) L3 target ⊆ fresh candidates → `fresh_ahead_dispatch`, never `genuine_wrong`; ceiling
moves only with dated per-site adjudication; (3) L3 target NOT in the candidate set → ABORT/scope out that site;
(4) any candidate not source/catalog-visible → ABORT/scope out; (5) `genuine_wrong > 0` → regression, no ratchet motion.

**T3 — inertness defined precisely (Important):** edge set + classifications byte-identical; NO new enum-string values
emitted; schema/report output either byte-identical (zero-count fields skipped via `skip_serializing_if`) or explicitly
additive-with-expected-zero — audit the serde/aldump/BC-Brain surfaces; never rely on compiler exhaustiveness for
hand-built JSON.

**T4/T5 — both-ways metric reporting (Important):** for THIS arc, report BOTH the new `real_unknown_rate` (excluding
`AmbiguousResolved`) AND a legacy/advisory rate including it; the CHANGELOG/charter addendum states plainly that these
edges remain PRACTICALLY unresolved at runtime from the tooling's perspective (a closed candidate set, not a pick) — no
stat-juking appearance. `ambiguous_resolved` gets its own histogram count + ratchet.

**PREFLIGHT (moved from T5, Important):** the read-only `.dependencies` special-casing audit runs BEFORE T1 (if any walker
excludes those folders, all CDO baselines are wrong — fix + rebaseline first; expected finding: none, per the prior quick
grep). T5 closes/documents it.

## Global Constraints

- `rustfmt <file>` per-file — NEVER `cargo fmt`. Stage only named files — NEVER `git add -A`. `CHANGELOG.md` per task.
  CI gates: `cargo clippy --release --all-features -- -D warnings` (NO `--tests`), `cargo fmt --check`,
  `cargo test --workspace` (no CDO_WS, green).
- **Soundness cardinal:** candidate routes must never overstate reachability (`fires_by_default=false` uniformly via the
  new Condition; default traversal + best-evidence scoring exclude them — verify `Histogram`'s scoring loop `continue`s);
  a collapse-marked candidate never yields a false Opaque/Source; the aliased-id footgun is closed BEFORE candidate sets
  are built (sequencing).
- **Honest labels:** never reuse ManualBinding/Polymorphic; the new docs state exactly what AmbiguousDispatch claims.
- **Measure, don't assume:** T4 Step 0 partitions; every reclassified edge exhaustively adjudicated (the candidate set
  verified against source for a sample incl. EVERY distinct emission pattern); per-site bijection for the taxonomy-only
  task (T3 = zero behavior change).
- Determinism; additive-only export changes; ratchets with dated notes.
- **Out of scope:** the 13 workspace-tier MemberNotFound (deferred, with the preprocessor-fixture prerequisite recorded);
  arg-type dispatch (picking among candidates); XmlPort/Query; unquoted bare fields; protected Variables[].

## Tasks

### Task 1: Source-overload alias marker (the fast fail-closed net — ABI-pattern mirror)
**Files:** `src/program/build.rs` (dedup marks), `node_extract.rs` (field), `resolver.rs` (`emit_event_flow_edges` guard);
fixtures + gates.
- [ ] Step 1: failing tests — (a) two same-id different-`param_sig_key` source decls → BOTH survive AND both carry
  `source_overload_aliased=true`; (b) an aliased PUBLISHER pair: `emit_event_flow_edges` must not emit a corrupted-span
  edge (skip or synthetic zero-span per the existing SymbolOnly-absent branch pattern — decide + document); (c) CONTROL:
  a true re-parse duplicate (same param_sig_key) still collapses unmarked; (d) single-publisher-sibling case unchanged
  (the existing `compound_obj_dup_and_overload_subscription...` fixture stays green).
- [ ] Step 2: run — fail. Step 3: implement (marker set in `dedup_routines_preserving_genuine_overloads` where both
  survive; the event guard). Step 4: CDO gates — expect byte-identical 151/0.83% (the 8 aliased pairs: verify whether any
  is a dual-publisher — record; `genuine_wrong=0`). Step 5: gates + commit
  `fix(resolve): mark aliased source overloads + guard event-publisher span attribution (Task 1)`.

### Task 2: Real source `sig_fp` (identity fix — centralize first)
**Files:** new shared constructor consuming the 5 sites (`node_extract.rs:452`, `body_map.rs:65`, `full.rs:210/:573`,
`stub.rs:68`); the fnv1a fingerprint; test-fixture audit; gates.
- [ ] Step 1: failing tests — (a) the 5 sites produce IDENTICAL ids for the same decl (a parity test through the shared
  fn); (b) two param-type-differing overloads get DISTINCT sig_fp (and distinct ids end-to-end: two graph nodes, two
  BodyMap entries, correct per-overload caller attribution on their outgoing edges); (c) params.is_empty() → 0 (ABI
  convention parity); (d) the T1 marker now only fires for true same-signature duplicates.
- [ ] Step 2: run — fail. Step 3: implement — ONE shared `source_routine_node_id(...)` used by all 5 sites; sig_fp =
  fnv1a over length-delimited lowercased param type texts (reuse the `param_sig_key` normalization; the
  `abi_ingest::param_type_fp` primitive). Audit the `#[cfg(test)]` `sig_fp:0` literals that run REAL parsed AL. Step 4:
  CDO gates — dispatch outcomes should be UNCHANGED (aliased pairs were already 2 candidates; now 2 distinct ids —
  confirm byte-identical 151/0.83% breakdown OR adjudicate any diagnostic-label shifts); semantic goldens must NOT move
  (site keys don't encode sig_fp — verify); frozen trigger/event digests — the 8 pairs' publisher spans may CORRECT
  (adjudicate as fidelity fixes, document). Step 5: gates + commit
  `feat(resolve): real source sig_fp via shared RoutineNodeId constructor — distinct overload identity (Task 2)`.

### Task 3: The taxonomy trio (mechanics only — zero behavior change)
**Files:** `edge.rs` (Condition/DispatchShape/ObligationOutcome + classify branch + Histogram field), `full.rs`
(completeness_for_shape + the duplicate histogram), `graphify_export.rs` (arms), `aldump.rs` (THE MANUAL JSON KEY),
harness invariants.
- [ ] Step 1: failing unit tests — the classify branch (all-AmbiguousDispatch → AmbiguousResolved; mixed → existing
  outcomes unchanged; the ConditionalResolved/ManualBinding path untouched); completeness mapping (AmbiguousOverload →
  Complete); the export strings; the Histogram field + BOTH histogram copies.
- [ ] Step 2: run — fail. Step 3: implement. NOTHING emits the new shapes yet: Step 4 = full workspace + CDO
  byte-identical (151/0.83%, goldens untouched — this task is inert by construction; if anything moves, STOP). Step 5:
  gates + commit `feat(resolve): AmbiguousDispatch/AmbiguousOverload/AmbiguousResolved taxonomy — inert mechanics (Task 3)`.

### Task 4: Candidate-carrying `resolve_in_object` + the metric-definition change
**Files:** `resolver.rs` (the tuple signature + 7 sites + the Interface extend), fixtures, charter/CHANGELOG/ratchets.
- [ ] **Step 0 (measure first):** partition the 56 by emission site on CDO (resolve_in_object `_` arm vs table-scope
  Ambiguous vs interface `matching!=1`) + confirm the fresh_extra hypothesis (what does the frozen golden hold for these
  sites?). The reclassification scope = the same-object `resolve_in_object` population ONLY; document the partition +
  the decision for the others (stay Unknown this plan unless the partition shows a trivially-honest case).
- [ ] Step 1: failing fixtures — (a) a genuine 2-overload call site → `AmbiguousResolved`, TWO candidate routes (correct
  ids post-T2), each `fires_by_default=false`, `SetCompleteness::Complete`, excluded from default traversal + best-evidence
  scoring; (b) a collapse-marked candidate inside the set → an Unresolved AbiCollapsedOverload-flavored route, never
  Opaque/Source; (c) the Interface-nesting case (an implementer with its own overload fan-out inside a Polymorphic edge)
  — design-reviewed behavior, pinned; (d) NEGATIVES: single-candidate unchanged; access-filtered/arity-mismatch shapes
  stay their T2-split reasons; applicability gates stay green (the new shape falls through `_ => {}`).
- [ ] Step 2: run — fail. Step 3: implement. Step 4: CDO — the same-object population leaves `unknown` (rate drops;
  record exact); EXHAUSTIVE adjudication of every reclassified edge (candidate sets verified against source for every
  distinct pattern); audit gate: `genuine_wrong=0`; fresh_extra/fresh_wrong movement adjudicated per the grounded
  hypothesis (zero-margin ceiling — dated note if it moves). Ratchets re-derived; **the charter §5/§8 + edge.rs docs +
  CHANGELOG metric-definition addendum land IN THIS COMMIT.** Step 5: gates + commit
  `feat(resolve): candidate-carrying AmbiguousResolved for same-object overload ambiguity — metric-definition change (Task 4)`.

### Task 5: The `.dependencies` audit (user-requested) + measure + close
- [ ] Step 1: the audit — sweep ALL program parts for `.dependencies` folder special-casing: every source walker
  (snapshot/provider.rs, legacy L2/L3 walkers, engine/deps), path/glob/string matching on folder names, scripts/, docs/,
  test fixtures, aldump path filters. Record findings (expected: none — a prior quick grep found only field accessors);
  fix anything found (they are NORMAL AL source — the user correction + memory note).
- [ ] Step 2: full re-measure (all gates); adjudication sign-off; ratchets at the floor; CHANGELOG (the arc + the
  metric-definition change restated + DEFERRED: the 13 w/ the preprocessor fixture prerequisite, arg-type dispatch,
  the remaining Unknown-partition populations); charter memory + MEMORY.md. Commit
  `docs(resolve): sig_fp identity + AmbiguousResolved complete — real-unknown 0.83%→X% (Task 5)`.

## Roadmap — beyond this plan
The 13 workspace-tier proven-absent design (starts with the `#if UNDEFINED ... procedure ... #endif` fixture); arg-type
dispatch (picking among carried candidates); the cross-object table-scope + interface-arm ambiguity populations (per the
T4 partition); unquoted bare implicit-Rec fields; protected Variables[]; Sender param-TYPE.
