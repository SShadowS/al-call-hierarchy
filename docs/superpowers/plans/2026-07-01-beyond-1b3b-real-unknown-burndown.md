# Beyond-1B.3b — Real-`unknown` Burn-Down + False-Confidence Audit Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

> Status: **v2.1** — round 2 (gpt-5.5 GO-WITH-CHANGES + gemini-3.1-pro GO-WITH-CHANGES, both "no v3 review
> needed") guardrails folded in: Task 1 allows a justified false-`Catalog`→honest-`Unknown` rate rise (gate on
> divergence + intrinsic fixtures, not rate) with exact source-candidate cardinality + a qualified-intrinsic
> (`System.X`) shadowing-bypass fixture; Task 2 detects the collision from the PRE-INTERNED extraction stream;
> Task 3 keys overrides by span/ordinal (not line) + a testable non-circularity invariant (adjudicator never
> calls the fresh resolver); Task 4 adds the `objects_by_id` numeric index + the single shared fail-closed
> `resolve_object_ref` helper (exact contract) that Tasks 5–7 call; Task 7 carries the resolved `ObjectNodeId`
> mechanically via a real `id` field (not by comment).
>
> v2 — rewritten after gpt-5.5 NO-GO + gemini-3.1-pro NO-GO (round 1). Convergent fixes
> folded in:
> 1. **No self-cementing golden.** Never overwrite the frozen L3 golden (`cdo-anon.json`) with fresh's
>    own output. Adjudicated corrections live in a SEPARATE `adjudicated-overrides.json`, overlaid
>    in-memory at audit time; the audit tests fresh AGAINST the adjudicated expected target (fresh is
>    the subject under test, never the data source).
> 2. **Lookup precedence is a root-cause engine fix, not a text comparison.** A visible source routine
>    of matching name+arity SHADOWS a same-named builtin (AL semantics: Source > Catalog). Adjudicating
>    `genuine_wrong` by comparing source token to `fresh_builtin` would LAUNDER a shadowing bug into the
>    golden. So: (Task 1) fix `resolve_member`/bare resolution to honor source-shadows-catalog + make the
>    builtin catalog STRUCTURALLY keyed (name + arity + receiver-kind), fail-closed — BEFORE adjudicating.
> 3. **The same-arity source-overload node COLLISION is a prerequisite, not deferred.** Two same-arity
>    source overloads collapse to one `RoutineNodeId` (source `sig_fp=0`); once Tasks 5–7 type `Rec`, the
>    receiver fix would emit a confident `Source` edge to whichever routine survived the collapse — an
>    honest `Unknown` turned into a false `Source`. Task 2 makes that path fail-closed (ambiguity-aware,
>    never pick-first) before any receiver consumer runs. (Full arg-type DISPATCH stays deferred.)
> 4. **Receiver inference is fail-closed + dependency-topology-aware** and carries the resolved
>    `ObjectNodeId` (no re-resolve-by-name). Decline (honest `Unknown`) on ambiguity, out-of-closure,
>    numeric-id-unresolved, Report dataitem scope, deep `CurrPage` chains, control-vs-subpage ambiguity.
> 5. **Rate-drop is never the correctness proof.** Each receiver task adds a resolution-delta correctness
>    gate (a hand-adjudicated CDO delta sample + the semantic-audit DIVERGENCE count must stay flat-or-down)
>    on top of the aggregate ratchet.

**Goal:** Drive the CDO primary real-`unknown` rate below its current 6.46% by SOUNDLY resolving the
three receiver-typing gaps, fix the lookup-precedence root cause behind the 42 `builtin-catalog-fp-collision`
divergences, and convert that 42-entry `genuine_wrong` manifest from an *unconfirmed* whitelist into a
*precedence-adjudicated, source-identity-verified* verdict overlay — **adding zero false-`Source`/`builtin`
claims** (the cardinal sin; charter §6, §8).

**Architecture:** Two soundness fixes land first (Task 1 lookup precedence + structural builtin catalog;
Task 2 fail-closed same-arity overload guard) because both the adjudication (Task 3) and the receiver
consumers (Tasks 5–7) depend on them. Task 3 adjudicates via an in-memory OVERLAY (never mutating the L3
golden). Task 4 promotes IR object properties into `ObjectNode` (additive node fidelity, charter §3).
Tasks 5–7 wire fail-closed, topology-aware receiver inference to consume them, each proven by a synthetic
fixture with EXACT route assertions AND a CDO before/after correctness gate. Task 8 ratchets per-bucket
counts, asserts the divergence gate, and writes the honest CHANGELOG. The clean-room **reference** for the
receiver gaps and lookup precedence is the legacy L3 engine (`src/engine/l3/`) — port the approach over the
IR + `ProgramGraph`/`ResolveIndex`, never import `engine::l3`.

**Tech Stack:** Rust (edition 2024, toolchain 1.96.0). No new dependency. No `engine::l3`/`engine::l2`
import in `src/program/resolve` (the 1B.3b invariant; only `builtins.rs::global_builtins` data catalog —
enforced by a new grep-guard test).

**Source of truth:** the four grounding reports (this session, all file:line-verified) + the charter
(`docs/superpowers/specs/2026-06-28-bc-semantic-intelligence-charter.md` §3 node-fidelity, §5 taxonomy,
§6 principles, §8 metric) + the two round-1 reviews. Read the charter §3/§5/§6/§8 first.

## Key facts grounding this plan (all file:line verified)

**Metric surface (L3-free, the dial this plan moves):**
- `aldump --program-call-graph-stats <ws>` → `resolve_full_program` (`src/program/resolve/full.rs:624`).
  Taxonomy = `ObligationOutcome` {`Resolved`,`ConditionalResolved`,`HonestDynamic`,`HonestEmpty`,`Unknown`}
  (`src/program/resolve/edge.rs:233-247`); `classify_obligation` (`edge.rs:250-286`); `real_unknown_rate`
  (`edge.rs:291-300`); `Histogram::of_edges` (`edge.rs:339-384`).
- CDO ceiling test: `cdo_full_program_coverage_and_self_reported_metric` (`tests/program_resolve_harness.rs:1101`),
  live recompute, asserts `primary_rate <= 0.07` (`:1179`); recorded baseline **6.46%** (`:1177`).
- Semantic-audit divergence/completeness: `run_cdo_semantic_audit` (`src/program/resolve/semantic_golden.rs:1921`),
  `FRESH_MISSING_CEILING = 191` (`tests/program_resolve_harness.rs:1666`) = page_rec=115 + codeunit_implicit_rec=24
  + trigger=38 + other=14 (compound ≈47 in a separate counter). `fresh_wrong`/`genuine_wrong` = fresh-vs-golden
  divergence (the soundness signal to keep flat-or-down).
- Env: `CDO_WS="U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud"`; `ENFORCE_CDO_WS=1` hard-fails on
  missing workspace/golden/zero-comparison (`cdo_ws_or_enforce()` `tests/program_resolve_harness.rs:1222`).

**Lookup precedence + builtin catalog (Tasks 1, 3):**
- `resolve_member` Record arm (`src/program/resolve/resolver.rs:665-716`) is **catalog-FIRST** then table
  procedures, then `member_unknown_route()` (`:676-678`, `:549-559`). A user/source table method whose name
  equals a builtin is therefore mis-classified `builtin` — a lookup-precedence bug (AL: source member shadows
  the intrinsic). Bare-call resolution (`resolve_bare` `resolver.rs:243-251`) similarly must prefer a visible
  source routine over a same-named intrinsic.
- Builtin catalog: `src/program/resolve/builtins.rs::global_builtins` — keyed by a **u64 fingerprint**
  (`param_type_fp`/`fnv1a` family, `src/program/abi_ingest.rs:23-42`). A hash collision (two different names →
  same u64) yields a false `builtin`. The catalog match must be verified structurally (name + arity +
  receiver-kind), fail-closed.

**genuine_wrong manifest (Task 3):**
- `tests/goldens/semantic-edges/known-genuine-divergences.json` — exactly 42 entries, shape
  `{unit, line, callee_fp, fresh_builtin, cause}`, **all** `cause = "builtin-catalog-fp-collision"`. The file's
  own `description` admits directionality is UNCONFIRMED. Split predicate `is_fresh_ahead_dispatch_anon`
  (`semantic_golden.rs:1095-1149`) is **symmetric by design** — it cannot say who is right.
- Gates: set-membership `cdo_l3_semantic_audit_no_fresh_wrong` (`tests/program_resolve_harness.rs:1617-1632`);
  unconditional `committed_goldens_metadata_is_valid` (`:1855`, `assert_eq!(manifest_entries.len(), 42)` `:1928`).
- Frozen golden `tests/goldens/semantic-edges/cdo-anon.json` encodes **L3's** per-site target. Minted by the
  dev-only `src/bin/mint-goldens.rs` + `src/program/l3_mint.rs` (sole sanctioned L3 consumer; `REGEN_TEMP_GOLDENS`-gated).

**Overload node collision (Task 2):**
- `RoutineNodeId { object, name_lc, enclosing_member_lc, params_count, sig_fp }` (`src/program/node.rs:107-118`);
  source-bearing routines always get `sig_fp = 0` (ABI-only routines get a real `param_type_fp`). Two same-arity
  source overloads ⇒ identical `RoutineNodeId` ⇒ one node (collision at index build).
- `ResolveIndex.routines_by_obj_name: (ObjectNodeId, name_lc) → Vec<RoutineNodeId>` (`src/program/resolve/index.rs:88-89`).
- `resolve_in_object` arity match (`src/program/resolve/resolver.rs:203-211`) does `candidates.find(|r| r.params_count == arity)`
  → **pick-first, no ambiguity guard** (TODO at `:207`). Contrast: the interface path (`resolver.rs:856-884`)
  correctly declines `>1` same-arity candidates to `Unresolved`.
- `RawSiteV2` (`src/program/resolve/extract.rs:56-72`) carries `arity` only — **no argument types**. Full
  arg-type dispatch is therefore impossible now and stays deferred; Task 2 only makes the path fail-closed.

**Receiver-gap mechanism (Tasks 4–7) — fresh engine `src/program/resolve/`:**
- `ObjectNode` (`src/program/node_extract.rs:32-40`) = `{id, name, declared_id, extends_target, implements,
  tier}` — **no** `source_table`/`table_no`/`page_controls`/`is_temporary`. `extract_nodes` (`:62-90`) never
  reads `obj.properties`/`obj.page_controls`.
- IR carries everything (`crates/al-syntax/src/ir/decl.rs`): `ObjectDecl.properties: Vec<ObjectProperty{name(lc),
  value, origin}>` (`:19,66`); `page_controls: Vec<PageControl{name, kind("part"/"systempart"/"usercontrol"),
  target}>` (`:35,46`); `report_dataitems: Vec<(name, source_table)>` (`:20-26`).
- `infer_receiver_type` (`src/program/resolve/receiver.rs:361-456`); `infer_implicit_rec` (`:464-492`):
  Page/PageExt/Report/ReportExt implicit Rec → `ReceiverType::Record{table:None}`; Codeunit → `ReceiverType::Unknown`.
  `ReceiverType::Record{table: Option<ObjectNodeId>}` (`:163`) — builtins resolve table-independently; only a
  **non-builtin** method on `table:None` becomes `Unknown`. Object-name → `ObjectNodeId` resolution already
  exists for the declared-var Record path (step 2, `receiver.rs:408-429`, via `ResolveIndex`).
- Member call site `resolve_call_site_obligation` `CalleeShape::Member` (`src/program/resolve/full.rs:312-333`)
  lowercases the **whole** receiver text (`:316`) — no dot-splitting anywhere in `src/program/resolve/`.
- **Clean-room L3 reference (port the approach, never import):** `source_table_name`/`source_table_temporary`
  (`src/engine/l3/l3_workspace.rs:40-41`, populated `:563`/`:621`); codeunit `TableNo`
  (`src/engine/l2/ir_walk.rs:1834`); `currpage_control_receiver` (`src/engine/l3/receiver_type.rs:741-825`
  — strips `CurrPage.`/optional trailing `.Page`, resolves Part/SystemPart → subpage Page, **declines >1 dot**);
  `page_controls_for` (`src/engine/l3/symbol_table.rs:244-258`, **merges a PageExtension's base-page controls**).
  **TestRunner implicit Rec is unhandled even in L3** (zero `TestRunner` hits in `src/`).

## Global Constraints

- Rust edition 2024; toolchain 1.96.0. `rustfmt <file>` per-file — **never** `cargo fmt`. Stage only named
  files — **never** `git add -A`. Update `CHANGELOG.md` (Keep-a-Changelog) for every task.
- CI gates (all must pass): `cargo clippy --release --all-features -- -D warnings` (NO `--tests`),
  `cargo fmt --check`, `cargo test --workspace` (fixture-only, **no `CDO_WS`** — fully green without the
  workspace). New synthetic fixtures live under `tests/r0-corpus/` and run in the no-CDO suite.
- **Soundness is the cardinal rule (charter §6, §8).** A receiver typed to the wrong table, a `CurrPage.<part>`
  bound to the wrong subpage, a pick-first overload, or a false `builtin` are all false-positive claims and are
  WORSE than `Unknown`. **Fail closed:** decline to honest `Unknown`/`Dynamic`/`ambiguous` on ANY shape the port
  does not provably cover. Never fabricate a resolution to move the metric.
- **Lookup precedence (AL semantics):** a visible source/ABI routine of matching name+arity shadows a same-named
  intrinsic — Source/ABI > Catalog. Object resolution is **dependency-topology-aware** (resolve by object KIND,
  respect namespace + dep closure + shadowing; numeric object-id distinct from name; decline on ambiguity /
  out-of-closure). A receiver that resolves to an `ObjectNodeId` CARRIES that id forward (no re-resolve-by-name).
- **No self-cementing.** The frozen L3 golden (`cdo-anon.json`) is NEVER overwritten with fresh output.
  Adjudicated corrections are a separate `adjudicated-overrides.json` (source-identity-verified, precedence-checked),
  overlaid in-memory; the audit tests fresh against the overlay. Charter §6 principle 6 (no proving ourselves with
  ourselves) holds.
- **Correctness > rate.** Each resolution-changing task: (a) a synthetic fixture asserting the EXACT route at the
  EXACT site (dispatch shape, route count, evidence, target id) + negative assertions; (b) WITH `CDO_WS`, the
  semantic-audit DIVERGENCE count (`fresh_wrong`/`genuine_wrong`) stays **flat-or-down** and a hand-adjudicated
  delta sample of newly-`Resolved` sites is correct; (c) the aggregate real-`unknown` rate as a ratchet only.
- **Node fidelity (charter §3):** promote `temporary` as `is_temporary: bool` (not stripped-and-discarded);
  carry object refs structurally (raw + normalized + numeric-id), not a lossy lowercased string.
- **Determinism (charter §C8):** node-property ordering + receiver resolution deterministic run-to-run; no
  `HashMap`-iteration nondeterminism in serialized/compared output.
- **No new oracle:** `src/program/resolve` stays `engine::l3`/`engine::l2`-free except `builtins.rs::global_builtins`
  — Task 8 adds a grep-guard test asserting this over `src/program/resolve/**`.
- **Out of scope (deferred to a focused follow-up plan):** full same-arity-type overload DISPATCH (needs source
  `sig_fp` + arg-type capture in `RawSiteV2` + type-match — Task 2 here only makes the path fail-closed); Report
  implicit-`Rec` (needs dataitem block-scope context); `fresh⊆l3` partial-recall validation; the snapshot
  double-include root cause; trigger-events as EventFlow; BindSubscription activation.

## File / module structure

| File | Responsibility |
|------|----------------|
| `src/program/resolve/resolver.rs` (modify) | Task 1: source-shadows-catalog precedence in member + bare resolution. Task 2: ambiguity-aware `resolve_in_object`. Tasks 5–7 consume receiver types. |
| `src/program/resolve/builtins.rs` (modify) | Task 1: structural (name+arity+receiver-kind) catalog match, fail-closed. |
| `src/program/resolve/index.rs` (modify) | Task 2: detect same-arity source-overload collisions at build; mark the slot ambiguous. Task 4: `objects_by_id` numeric index + the shared fail-closed `resolve_object_ref` helper. |
| `tests/goldens/semantic-edges/adjudicated-overrides.json` (create) | Task 3: precedence-adjudicated, source-identity-verified expected-target overlay. |
| `tests/goldens/semantic-edges/known-genuine-divergences.json` (modify) | Task 3: per-entry `verdict` + `callee_text` + `source_sha`; invariant assertions. |
| `src/program/resolve/semantic_golden.rs` (modify) | Task 3: overlay load + adjudicated-vs-fresh audit. |
| `src/program/node_extract.rs` (modify) | Task 4: `source_table`/`table_no`/`page_controls`/`is_temporary` (structured) from IR. |
| `src/program/resolve/receiver.rs` (modify) | Tasks 5–7: fail-closed implicit-Rec + `CurrPage.<part>.Page` Step 0. |
| `tests/r0-corpus/**` (create) | Tasks 1–7: synthetic fixtures incl. negatives (shadowing, cross-app same-name, numeric-id, control-vs-subpage, deep-chain). |
| `tests/program_resolve_harness.rs` (modify) | Tasks 1–8: fixture + CDO correctness gates + tightened per-bucket ratchets + grep-guard. |
| `CHANGELOG.md` + charter memory (modify) | Task 8. |

---

### Task 1: Lookup precedence (Source shadows Catalog) + structural builtin-catalog match

**Files:** Modify `src/program/resolve/resolver.rs` (member + bare resolution order), `src/program/resolve/builtins.rs`
(structural match); Test `tests/r0-corpus/ws-builtin-shadow/` + no-CDO harness. **Root-cause fix behind the 42
`builtin-catalog-fp-collision` divergences; precedes adjudication (Task 3).**

- [ ] **Step 1: Write failing fixtures** — `tests/r0-corpus/ws-builtin-shadow/`: (a) a Table `Acme` with a
  user procedure `StrLen(): Integer`; caller does `var R: Record Acme; R.StrLen()` — assert it resolves to
  `Acme.StrLen` with `Evidence::Source`, NOT `builtin` (today: catalog-first → false `builtin`). (b) a Codeunit
  with a local `procedure Error(Msg: Text)`; caller does bare `Error('x')` — assert it resolves to the LOCAL
  `Error` (`Evidence::Source`), not the intrinsic. (c) **genuine-builtin regression** (must STAY `Catalog`): a
  record builtin `R.SetRange(...)`, a global builtin bare `Message('x')`, and a framework-member builtin (e.g.
  a `JsonObject` method) — each with NO source competitor → `Evidence::Catalog`. (d) a fabricated fp-collision
  (a call whose `callee_fp` matches a catalog entry but whose NAME differs) — assert it is NOT classified `builtin`
  (falls through to normal resolution / honest `Unknown`). (e) **qualified intrinsic bypass** (gemini round-2): a
  Codeunit defining a local `procedure Message(...)` AND a caller using the FULLY-QUALIFIED platform call
  (`System.Message(...)` or the equivalent qualified-intrinsic syntax the IR records) — assert the qualified call
  binds to the intrinsic `Catalog`, the local Source does NOT shadow it (qualified platform calls bypass the
  source-shadowing check).
- [ ] **Step 2: Run — fail** ((a),(b),(d) misclassify as builtin).
- [ ] **Step 3: Implement precedence** — in `resolve_member` Record arm (`resolver.rs:665-716`) and bare
  resolution (`resolve_bare` `:243-251`): look up SOURCE/ABI candidate(s) of matching name+arity FIRST, with
  EXACT cardinality semantics (gpt round-2): **exactly one** visible source/ABI candidate → `Source`/`ABI`;
  **more than one** → honest **ambiguous/`Unknown`** (source ambiguity STILL shadows the catalog — never let
  "multiple source candidates the resolver cannot choose" fall through to a false intrinsic); **zero** → consult
  the builtin catalog; else `Unknown`. **Qualified-intrinsic bypass** (gemini round-2): when the call is parsed as
  a fully-qualified platform intrinsic (e.g. `System.<name>`), skip the source-shadowing check and bind to the
  `Catalog` directly. Preserve the existing table-independent-builtin behavior ONLY where no source member shadows.
- [ ] **Step 4: Implement structural catalog match** — in `builtins.rs::global_builtins` lookup: after the u64
  fingerprint match, VERIFY the canonical method NAME (and arity + receiver-kind where the catalog records them);
  on mismatch, fail closed (do not classify `builtin`). Add name/arity/receiver-kind to the catalog key or a
  post-match guard. This makes a hash collision impossible to surface as a false `builtin`.
- [ ] **Step 5: Run — pass** (all fixtures). Then (WITH `CDO_WS`) run `cdo_full_program_coverage_and_self_reported_metric`
  + `run_cdo_semantic_audit`. Acceptance (gpt round-2 — structural validation may correctly turn a previously
  false `Catalog` into honest `Unknown`, so the rate MAY rise as a soundness correction): **(i)** the
  divergence/false-confidence count must NOT rise (the precedence fix should REDUCE false-`builtin` divergences —
  some of the 42 resolve to Source now); **(ii)** the genuine-builtin regression fixtures (Step 1c) stay `Catalog`;
  **(iii)** any real-`unknown` increase must be explained as false-`Catalog` → honest-`Unknown` and is ratcheted as
  the justified new floor in Task 8 (NOT a regression). Record the delta with its justification.
- [ ] **Step 6: rustfmt + clippy + (no-CDO) `cargo test --workspace` + commit** — `fix(resolve): source shadows
  builtin (lookup precedence) + structural builtin-catalog match, fail-closed (beyond-1B.3b Task 1)`.

---

### Task 2: Fail-closed same-arity source-overload guard (node soundness, prerequisite to Tasks 5–7)

**Files:** Modify `src/program/resolve/index.rs` (collision detection at build), `src/program/resolve/resolver.rs`
(ambiguity-aware `resolve_in_object`); Test `tests/r0-corpus/ws-overload-collision/` + no-CDO harness. **Full
arg-type DISPATCH stays out of scope — this only prevents a confident-wrong `Source` to a collapsed node.**

- [ ] **Step 1: Write failing fixtures** — `tests/r0-corpus/ws-overload-collision/`: a Table/Codeunit with two
  same-name same-arity SOURCE overloads differing only by param type (`procedure Resolve(x: Integer)` /
  `procedure Resolve(x: Code[20])`); a caller invoking `Resolve(...)`. Assert the call does NOT resolve to a single
  confident `Source` target — it is honest **ambiguous/`Unknown`** (`DispatchShape` per the taxonomy; never a
  guessed pick-first route). Also assert the graph did not silently DROP one overload (both are represented or the
  slot is explicitly marked ambiguous). Add a single-overload control case that still resolves cleanly.
- [ ] **Step 2: Run — fail** (current pick-first emits a confident route / one overload is dropped).
- [ ] **Step 3: Implement** — (a) detect the collision from the **pre-interned routine extraction stream** (gpt
  round-2): the `(object, name_lc, arity)` slot must be marked AMBIGUOUS at the point where both raw routines are
  still visible — BEFORE any node interning/dedup can collapse them and erase the evidence. If `index.rs` populating
  `routines_by_obj_name` already sees both raw routines, mark there; otherwise preserve a collision flag/counter
  during `node_extract` and propagate it. Never let one silently overwrite the other. (b) In `resolve_in_object`
  (`resolver.rs:203-211`), replace pick-first: when the matched slot is ambiguous (≥2 same-arity candidates and no
  arg-type evidence — which is always, today), return honest ambiguous/`Unknown` (mirror the interface path's
  `>1 → Unresolved` at `:856-884`); 1 candidate → resolve as before. Apply on every caller of `resolve_in_object`
  (bare `:243-251`, member object/self `:763-799`).
- [ ] **Step 4: Run — pass** (fixtures). Then (WITH `CDO_WS`) re-measure: real-`unknown` rate may rise SLIGHTLY if
  CDO has such collisions previously pick-first'd (that is a CORRECTION — false `Source` → honest `Unknown`);
  the divergence count must not rise. Record any rate change with its justification (soundness correction, not a
  regression — note it for the Task 8 ceiling).
- [ ] **Step 5: rustfmt + clippy + (no-CDO) test + commit** — `fix(resolve): fail-closed same-arity overload guard
  (no pick-first Source; collision-aware index) (beyond-1B.3b Task 2)`.

---

### Task 3: Source-adjudicate `genuine_wrong=42` via a precedence-checked OVERLAY (no golden mutation)

**Files:** Create `tests/goldens/semantic-edges/adjudicated-overrides.json`; modify
`tests/goldens/semantic-edges/known-genuine-divergences.json` (verdict + evidence), `src/program/resolve/semantic_golden.rs`
(overlay load + adjudicated audit), `tests/program_resolve_harness.rs`. **Depends on Tasks 1–2 (the engine fixes may
already resolve the shadowing subset).**

**Interfaces:** a CDO-gated adjudicator that, for each residual manifest entry, records a SOUND verdict from
INDEPENDENT criteria (not fresh's output): the call SHAPE (global/member/qualified), the receiver category, the
arity, whether the enclosing object/scope declares a same-name source member (lookup precedence), the structural
catalog candidate (name+arity+receiver-kind), and a `source_sha` (content hash of the adjudicated unit). Verdicts:
`l3_error_intrinsic` (no source competitor + structural catalog match holds → the call IS the intrinsic, L3 mis-resolved
it), `fresh_false_builtin` (a source competitor shadows, or shape/arity mismatch → fresh was wrong; should be fixed by
Tasks 1–2), `needs_manual_review` (any dimension unavailable — fail closed, NOT auto-passed).

- [ ] **Step 1: Re-run the audit after Tasks 1–2** (WITH `CDO_WS`) and capture the RESIDUAL `genuine_wrong` set —
  the shadowing/collision subset should now resolve to `Source` and drop out. Record the new count.
- [ ] **Step 2: Write the failing adjudication test** — `cdo_genuine_wrong_is_precedence_adjudicated` (CDO-gated):
  for each residual entry, derive the verdict from the independent criteria above (open `CDO_WS/<unit>`, verify
  `source_sha` matches the committed adjudication — charter §C2 source identity; if drift, FAIL not silently pass);
  assert every entry's manifest `verdict` matches the derived verdict, and assert ZERO `needs_manual_review` and
  ZERO `fresh_false_builtin` remain (any survivor is a real bug for Tasks 1–2 to absorb).
- [ ] **Step 3: Build the overlay** — for each `l3_error_intrinsic` site, write an `adjudicated-overrides.json`
  entry. **Site key (gpt round-2 — `unit + line` is NOT enough; multiple calls per line):** anonymized unit id +
  a stable call-site span OR per-file call ordinal + `callee_text` + `arity` + the old L3 target fingerprint /
  `callee_fp` as a disambiguator. **Expected target:** the ADJUDICATED structural catalog intrinsic, carrying its
  canonical name+arity+receiver-kind — derived from the lookup-precedence analysis, NOT copied from fresh's edge +
  `source_sha` + a human note. **Non-circularity invariant (gpt round-2, make it testable):** the adjudicator MUST
  NOT call `resolve_full_program`, compare against fresh's routes, or copy fresh edge/target ids — it uses only
  syntax facts, the source-symbol inventory, the structural builtin catalog, and `source_sha`. Overlay entries hold
  CANONICAL CATALOG KEYS / expected route FACTS, never serialized fresh edge ids (assert this in the metadata test).
  `cdo-anon.json` (the L3 golden) is left UNTOUCHED.
- [ ] **Step 4: Overlay the audit** — in `run_cdo_semantic_audit`/`semantic_golden.rs`: load `cdo-anon.json`, overlay
  `adjudicated-overrides.json` in-memory (override the L3 target for adjudicated sites), then diff fresh against the
  OVERLAID oracle. An adjudicated site now passes only if fresh matches the ADJUDICATED expected (independent), not
  merely "fresh == fresh". Sites still genuinely divergent stay in `genuine_wrong` and must be ⊆ the (shrunk) manifest.
- [ ] **Step 5: Manifest invariants** — replace the bare `assert_eq!(len, 42)` (`tests/program_resolve_harness.rs:1928`)
  with: every entry has `verdict` + `callee_text` + `source_sha`; no duplicate site keys; every `l3_error_intrinsic`
  has a matching `adjudicated-overrides.json` entry; count asserted as the new adjudicated total (with a comment
  recording the split N intrinsic / M false-builtin-fixed-by-Tasks-1-2 / 0 manual).
- [ ] **Step 6: Run** — (no CDO) `cargo test --workspace` green incl. invariants; (WITH `CDO_WS` + `ENFORCE_CDO_WS=1`)
  the adjudication + overlaid audit + self-metric green, deterministic, `checked_sites > 0`.
- [ ] **Step 7: rustfmt + clippy + commit** — `fix(resolve): precedence-adjudicate genuine_wrong via overlay (L3 golden
  untouched; source-identity-verified) (beyond-1B.3b Task 3)`.

---

### Task 4: Node fidelity + the shared, fail-closed `resolve_object_ref` helper

**Files:** Modify `src/program/node_extract.rs` (fields), `src/program/resolve/index.rs` (numeric id index +
`resolve_object_ref`); Test (unit tests, no-CDO). **Pure additive: fields + helper added; the receiver consumers
(Tasks 5–7) are the only callers, added later — zero resolution behavior change here. Enables Tasks 5–7.**

**Interfaces (Produces):**
- `ObjectNode.source_table: Option<ObjectRef>` and `ObjectNode.table_no: Option<ObjectRef>` where
  `pub enum ObjectRef { Name { raw: String, normalized_lc: String }, Id(i64) }` — so `SourceTable = "Sales Header"`,
  `SourceTable = 36`, and `TableNo = Item` are all represented losslessly (numeric id distinct from name).
- `ObjectNode.source_table_temporary: bool` — `true` when the `SourceTable` property carried `, temporary`
  (charter §3 node fidelity; do NOT discard).
- `ObjectNode.page_controls: Vec<PageControlNode>` where `pub struct PageControlNode { pub name_lc: String, pub
  kind: PageControlKind /* Part | SystemPart | UserControl */, pub target: ObjectRef }` — Page/PageExt, document order.
- `ResolveIndex.objects_by_id: HashMap<(ObjectKind, i64), ObjectNodeId>` (gemini round-2) — an O(1) numeric-id
  index respecting the SAME dependency-topology/closure scope as the existing `(kind, name_lc)` index, so
  `ObjectRef::Id(36)` resolves without an O(N) scan across 11k+ nodes (charter §E4).
- **`resolve_object_ref(from: ObjectNodeId, kind: ObjectKind, r: &ObjectRef) -> ObjectRefResolution`** (gpt round-2,
  the ONE shared helper Tasks 5–7 call) where
  `pub enum ObjectRefResolution { Unique(ObjectNodeId), Ambiguous, OutOfClosure, Unresolved }`. **Exact contract:**
  - `ObjectRef::Id(n)` matches a declared numeric object id of the SAME `kind` only (via `objects_by_id`).
  - `ObjectRef::Name` matches by `kind` within the AL-visible dependency closure of `from`'s app; workspace/source
    objects shadow dependency objects only where AL lookup rules say so; namespace-qualified vs unqualified names
    are distinct WHEN namespace data exists.
  - Candidate ordering is deterministic; `Ambiguous`/`OutOfClosure`/`Unresolved` NEVER return an id (fail-closed).
  - **If the current `ResolveIndex` lacks the app-identity/dependency/namespace data to PROVE uniqueness, return
    `Ambiguous`/`OutOfClosure`** (never improvise a flat lowercased global match) — Tasks 5–7 then stop at
    `Record{table: None}` / `Unknown`. Conservative-decline is sound; a guessed id is the cardinal sin.

- [ ] **Step 1: Write failing unit tests** — (a) IR `ObjectDecl`s: a Page `SourceTable = Customer` + `part(Lines;
  "Sales Line Subform")`; a Page `SourceTable = 36`; a Codeunit `TableNo = Item`; a Table (none) → assert the lowered
  `ObjectNode` carries `source_table == Some(Name{normalized_lc:"customer"})` / `Some(Id(36))`, one `page_controls`
  entry `{name_lc:"lines", kind:Part, target:Name{normalized_lc:"sales line subform"}}`, `table_no ==
  Some(Name{normalized_lc:"item"})`, `source_table_temporary` `false` (+ a `, temporary` case → `true`), Table all
  `None`/empty; document order preserved. (b) **`resolve_object_ref` tests:** `Id(n)` of the right kind → `Unique`;
  `Id(n)` of a wrong/absent kind → `Unresolved`; a `Name` unique in closure → `Unique`; a `Name` matching TWO
  objects (two apps, same kind/name) → `Ambiguous`; a `Name`/`Id` not in the closure → `OutOfClosure`/`Unresolved`;
  determinism across two builds.
- [ ] **Step 2: Run — fail** (fields/index/helper don't exist).
- [ ] **Step 3: Implement** — add the `ObjectNode` fields + `ObjectRef`/`PageControlNode`/`PageControlKind`. In
  `extract_nodes` (`:62-90`): scan `obj.properties` for `sourcetable` → `ObjectRef` (numeric → `Id`, else
  `Name{raw, normalized_lc}`; detect+strip a trailing `, temporary` → set `source_table_temporary`); `tableno` →
  `ObjectRef`; map `obj.page_controls` → `Vec<PageControlNode>` (kind→enum, target→`ObjectRef`); populate per kind
  only (SourceTable for Page/PageExt/Report/ReportExt; TableNo for Codeunit; page_controls for Page/PageExt). Build
  `objects_by_id` alongside the existing name index in `index.rs`. Implement `resolve_object_ref` to the contract
  above (reuse the existing closure/topology logic the name path uses; fail closed when uniqueness is unprovable).
- [ ] **Step 4: Run — pass.** `cargo test --workspace` (no CDO) fully green (no consumer yet → no behavior change).
- [ ] **Step 5: rustfmt + clippy + commit** — `feat(resolve): node fidelity (SourceTable/TableNo/page-controls/
  is_temporary, structured ObjectRef) + objects_by_id index + fail-closed resolve_object_ref (beyond-1B.3b Task 4)`.

---

### Task 5: `regression_page_rec` — Page/PageExt implicit `Rec` via `SourceTable` (fail-closed, ≈115)

**Files:** Modify `src/program/resolve/receiver.rs` (`infer_implicit_rec`); Test `tests/r0-corpus/ws-page-rec/`
+ CDO correctness gate. **Page/PageExtension ONLY — Reports are EXCLUDED (nested dataitem scope; stay honest
`Unknown`, a future task).**

- [ ] **Step 1: Write failing + negative fixtures** — `tests/r0-corpus/ws-page-rec/`: (a) Table `Customer` with
  `GetDisplayName()`; Page `SourceTable = Customer` whose trigger calls `Rec.GetDisplayName()` — assert it resolves
  to `Customer.GetDisplayName`, `Evidence::Source`, exact target id; a builtin `Rec.SetRange(...)` stays `builtin`.
  (b) **Negative:** a Page with NO `SourceTable` → `Rec.Foo()` stays honest `Unknown`. (c) **Negative:** two Tables
  named `Customer` in different apps/namespaces (cross-app ambiguity) → the page-rec resolution DECLINES to honest
  `Unknown` (must not pick one). (d) a `var Rec: Record OtherTable` local shadowing the implicit Rec → uses the
  declared type, not SourceTable. (e) a Report with a dataitem → `Rec.X()` stays honest `Unknown` (excluded).
- [ ] **Step 2: Run — fail** ((a) is `Unknown` today).
- [ ] **Step 3: Implement** — in `infer_implicit_rec` (`receiver.rs:464-492`), the Page/PageExtension arm: if
  `from_object.source_table` is `Some(ref)`, call `resolve_object_ref(from, Table, ref)` (Task 4): `Unique(id)` →
  `ReceiverType::Record{table: Some(id)}`; `Ambiguous`/`OutOfClosure`/`Unresolved` → `Record{table: None}` (builtins
  still resolve; non-builtins stay honest `Unknown`). For **PageExtension** with no own `SourceTable`, consult the
  extended base page's `source_table`. **Leave Report/ReportExtension returning `Record{table: None}`** (excluded —
  comment why). Keep the `var Rec` shadow check (step 2) ahead of this.
- [ ] **Step 4: Run — pass** (incl. all negatives). Then the CDO correctness gate (WITH `CDO_WS`):
  `cdo_full_program_coverage_and_self_reported_metric` + `run_cdo_semantic_audit` — real-`unknown` rate drops,
  `page_rec` `fresh_missing` drops (target 115 → near 0), **divergence count flat-or-down**, AND hand-adjudicate a
  sample of the newly-`Resolved` page-rec sites (open the CDO source, confirm the target table+method is correct).
  Deterministic across two runs.
- [ ] **Step 5: rustfmt + clippy + (no-CDO) test + commit** — `feat(resolve): resolve Page implicit-Rec via
  ObjectNode.source_table, topology-aware fail-closed (regression_page_rec) (beyond-1B.3b Task 5)`.

---

### Task 6: `regression_codeunit_implicit_rec` — Codeunit `TableNo` implicit `Rec` (fail-closed, ≈24)

**Files:** Modify `src/program/resolve/receiver.rs`; Test `tests/r0-corpus/ws-codeunit-rec/` + CDO correctness gate.

- [ ] **Step 1: Write failing + negative fixtures** — Table `Item` with `Recalculate()`; Codeunit `TableNo = Item`,
  `OnRun` calls `Rec.Recalculate()` → resolves to `Item.Recalculate`, `Evidence::Source`. **Negatives:** a Codeunit
  with no `TableNo` → `Rec.X()` honest `Unknown`; a `Subtype = TestRunner` codeunit → `Rec.X()` honest `Unknown`
  (no statically-typed implicit Rec); a cross-app ambiguous `TableNo` name → DECLINE.
- [ ] **Step 2: Run — fail** (Codeunit Rec → `Unknown` today).
- [ ] **Step 3: Implement** — in `infer_implicit_rec`, the Codeunit arm: if `from_object.table_no` is `Some(ref)`,
  call `resolve_object_ref(from, Table, ref)` → `Unique(id)` → `Record{table: Some(id)}`; else `Unknown`.
  **TestRunner** (`Subtype = TestRunner`): no static implicit-Rec table — leave honest `Unknown`/`Dynamic`, document
  the decline. Never fabricate.
- [ ] **Step 4: Run — pass** (incl. negatives). CDO correctness gate (WITH `CDO_WS`): `codeunit_implicit_rec` drop
  (target 24 → near 0 minus genuine TestRunner residual), rate down, divergence flat-or-down, sample hand-adjudicated,
  deterministic.
- [ ] **Step 5: rustfmt + clippy + (no-CDO) test + commit** — `feat(resolve): resolve Codeunit implicit-Rec via
  ObjectNode.table_no, fail-closed; TestRunner honest-declined (regression_codeunit_implicit_rec) (beyond-1B.3b Task 6)`.

---

### Task 7: `regression_compound_receiver` — `CurrPage.<part>.Page` subpage instance (control-aware, ≈47)

**Files:** Modify `src/program/resolve/receiver.rs` (Step 0 in `infer_receiver_type`); Test
`tests/r0-corpus/ws-compound-receiver/` + CDO correctness gate. **Resolve ONLY the subpage-instance shape
(`CurrPage.<part>.Page`); a bare control reference and SystemPart/UserControl are distinct — decline, don't fabricate.**

- [ ] **Step 1: Write failing + negative fixtures** — `tests/r0-corpus/ws-compound-receiver/`: subpage Page
  `CustomerCardPart` with `RefreshLines()`; host Page `part(Lines; CustomerCardPart)`. (a) `CurrPage.Lines.Page.RefreshLines()`
  → resolves to `CustomerCardPart.RefreshLines`, `Evidence::Source`, exact id. (b) **Negative — control vs subpage:**
  `CurrPage.Lines.Update(false)` / `CurrPage.Lines.Visible` (a CONTROL-property reference, no `.page`) must NOT be
  routed to the `CustomerCardPart` Page object — honest `Unknown` (or a distinct control receiver), since `Update`/`Visible`
  are control members, not subpage procedures. (c) **Negative — deep chain:** `CurrPage.Lines.Page.Foo.Bar()` stays
  honest `Unknown` (>1 remaining segment). (d) **Negative — unknown part:** `CurrPage.Nope.Page.X()` → `Unknown`.
  (e) **SystemPart/UserControl:** `CurrPage.<systempart>.X()` / `CurrPage.<usercontrol>.X()` → honest `Unknown`/decline,
  not a fabricated Framework route.
- [ ] **Step 2: Run — fail** ((a) is `Unknown` today; whole `"currpage.lines.page"` matches no arm).
- [ ] **Step 3: Implement Step 0** in `infer_receiver_type` (`receiver.rs:361`, before singletons): if `receiver_lc`
  starts with `"currpage."`, parse the remainder. ONLY when it is exactly `<part>.page` (one control segment +
  trailing `.page`): look up `<part>` in `from_object.page_controls` (for a PageExtension host, merge the extended
  base page's controls — mirror `symbol_table.rs:249-256`); if it is a `Part` whose `target` resolves via
  `resolve_object_ref(from, Page, target)` to `Unique(id)`, return the receiver CARRYING that id **mechanically**
  (gpt round-2 — not by comment): extend `ReceiverType::Object` to `Object { kind, name_lc, id: Option<ObjectNodeId> }`
  (existing constructions pass `id: None`), set `id: Some(page_id)` here, and have `resolve_member`'s Object arm
  short-circuit on a present `id` (skip re-resolve-by-name). For a bare `<part>` (no `.page`), a `SystemPart`, a
  `UserControl`, more than one remaining segment, an unknown part, or a non-`Unique` `target` → fall through to honest
  `Unknown` (do NOT fabricate a Page/Framework route).
- [ ] **Step 4: Run — pass** (incl. ALL negatives). CDO correctness gate (WITH `CDO_WS`): compound bucket drop
  (target ≈47 → near 0 for the single-segment `.page` shape), rate down, divergence flat-or-down, sample
  hand-adjudicated, deterministic.
- [ ] **Step 5: rustfmt + clippy + (no-CDO) test + commit** — `feat(resolve): resolve CurrPage.<part>.Page subpage
  receivers (control-aware, fail-closed) (regression_compound_receiver) (beyond-1B.3b Task 7)`.

---

### Task 8: Re-measure, per-bucket ratchets + divergence gate, grep-guard, honest CHANGELOG + charter memory

**Files:** Modify `tests/program_resolve_harness.rs` (ratchets + grep-guard), `CHANGELOG.md`, charter memory; Test.

- [ ] **Step 1: Full re-measure** (WITH `CDO_WS` + `ENFORCE_CDO_WS=1`): record the NEW primary real-`unknown` rate +
  `unknown` COUNT, `fresh_missing` count + per-bucket residual (page_rec/codeunit_rec/compound/trigger/other), and the
  `genuine_wrong`/divergence count. Capture the `aldump --program-call-graph-stats` histograms for the CHANGELOG.
- [ ] **Step 2: Tighten ratchets (counts, not just a float)** — lower `primary_rate <=` from `0.07`
  (`tests/program_resolve_harness.rs:1179`) to the new value + a tiny deterministic margin AND add a workspace
  real-`unknown` COUNT ceiling; lower `FRESH_MISSING_CEILING` (`:1666`) to the new residual with per-bucket
  breakdown; add a divergence-count ceiling (`genuine_wrong`/`fresh_wrong` must not exceed the post-adjudication
  value). Dated comments; ratchets never loosen. (If Task 2 raised the rate as a soundness correction, record that
  count explicitly as the justified new floor.)
- [ ] **Step 3: Grep-guard test** — add a no-CDO test scanning `src/program/resolve/**` that FAILS on any
  `engine::l3`/`engine::l2` import except `builtins.rs::global_builtins` (lock the 1B.3b invariant against new files).
- [ ] **Step 4: Run** — (no CDO) `cargo test --workspace` fully green incl. the grep-guard; (WITH `CDO_WS`) all CDO
  gates green under the tightened ratchets, deterministic, `checked_sites > 0`.
- [ ] **Step 5: CHANGELOG (honest)** — Fixed: Task 1 lookup-precedence (Source shadows Catalog) + structural catalog
  match; Task 2 fail-closed overload guard; Task 3 precedence-adjudicated `genuine_wrong` via overlay (L3 golden
  untouched; manifest 42 → K). Added/Changed: Tasks 4–7 closed page_rec/codeunit_rec/compound via charter §3 node
  fidelity, with before/after real-`unknown` rate (6.46% → X%) + count, `fresh_missing` (191 → Y). State plainly what
  stays honest (Report Rec, TestRunner, deep compound chains, cross-app-ambiguous tables) and that overload DISPATCH +
  `fresh⊆l3` recall are the next plan. No `engine::l3` import added (grep-guarded).
- [ ] **Step 6: Charter memory** — append a beyond-1B.3b entry to the charter memory
  (`C:\Users\SShadowS\.claude\projects\U--Git-al-call-hierarchy\memory\semantic-intelligence-charter.md`): real-`unknown`
  6.46% → X%, lookup-precedence fixed, genuine_wrong adjudicated via overlay, receiver-gaps closed fail-closed,
  overload-dispatch + Report-Rec the next plan. Update `MEMORY.md` pointer if needed.
- [ ] **Step 7: Commit** — `docs(resolve): beyond-1B.3b complete — precedence + receiver-gaps closed, genuine_wrong
  adjudicated; real-unknown 6.46%→X% (beyond-1B.3b Task 8)`.

---

## Roadmap — beyond this plan

Full same-arity-type overload **DISPATCH** (source-side `sig_fp` + arg-type capture in `RawSiteV2` + type-match,
keeping the genuinely-ambiguous `Variant` case honestly ambiguous — fixture at `tests/r0-corpus/ws-overload-arg-type/`);
**Report implicit-`Rec`** with dataitem block-scope context (nested dataitems → per-trigger source table);
`fresh⊆l3` partial-recall validation; the snapshot double-include root cause; trigger-events as EventFlow;
BindSubscription activation; cross-app effective-page-control sets (dependency-extension-added controls).
North-star (charter §8): drive the workspace-originated real-`unknown` rate to its provably-dynamic residual,
risk-weighted by centrality.
