# Report-dataitem receivers + .dependencies ingestion fix + Unknown reason-split Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

> Status: **v3** (round 2: gemini GO, gpt GO-WITH-CHANGES — surviving items folded below. **USER CORRECTION (BINDING,
> 2026-07-03): the former Task 1 (skip `.dependencies/` in ingestion) is DELETED — `.dependencies/` folders are NORMAL AL
> source (old CAL→AL conversion naming), no special meaning; excluding them would drop real source. All T1 text below is
> VOID; the 9/25 MemberNotFound sites there are honest workspace reality. Tasks renumbered: T1=dataitem receivers,
> T2=reason-split, T3=measure/close.** Round-2 folds still binding: scrub — `UndeclaredExternalTarget` is DROPPED
> (uniform `ObjectNotInGraph`); the access-narrowed label IS `AccessFilteredOverload` (decided); the dataset-vs-requestpage
> context must be an EXPLICIT additive IR field/flag (named in Task files; never repurpose existing semantics);
> `receiver_tier` is additive/nullable (no renames/reorders — BC-Brain consumes); ONE centralized quote-token helper shared
> by `receiver.rs` + `full.rs`, escape-aware, len>2 (the `""` empty-quote guard), unsupported forms fail closed;
> retained-site adjudication discipline from the old T1 addenda applies to any golden movement generally.) Seventh resolution arc (master `4921746`, CDO primary real-`unknown` 0.99% /
> `unknown=180`: CompoundReceiver 61, OverloadAmbiguous 56, UntrackedReceiver 37, MemberNotFound 25,
> BuiltinPrecedenceCollision 1; `genuine_wrong=0`; harness 128/128). Grounding (two reports, this session) RESHAPED the
> original intent: the OverloadAmbiguous/MemberNotFound → charter-§5 reclassification is NOT a relabeling exercise —
> OverloadAmbiguous conflates 4 emission shapes (only one genuinely ambiguous; candidate identities are DISCARDED before
> the route is built); MemberNotFound sampled 0/25 honestly-reclassifiable (9/25 = a real `.dependencies/` ingestion-scope
> bug polluting the graph with a stray decompiled cache at Workspace tier; 7/25 = an UNDECLARED out-of-snapshot app —
> open-world per charter A1, misclassified as proven absence; rest = dead references to absent objects). SO THIS PLAN:
> 1. **Fix the `.dependencies/` ingestion-scope bug** (a stray non-app folder ingested as Workspace tier — version-drift
>    noise in the denominator AND numerator).
> 2. **Model report-dataitem receivers** (~27 sites — the last real modeling lever; the IR already carries everything) +
>    fix two pre-existing defects it exposes: the naive dot-substring quote guard (5/16 real dataitem names have embedded
>    periods) and the ReportExtension `modify()` lowerer gap.
> 3. **Reason-SPLIT the conflated buckets** (diagnostic, count-preserving — the landed UnknownReason pattern): the data
>    that makes any FUTURE outcome-reclassification honest.
> 4. **DEFER the outcome-reclassification proper** (new ObligationOutcome + candidate-carrying routes) to its own plan,
>    now with real data — the genuinely-reclassifiable population is small and the design is heavy (Polymorphic is
>    semantically WRONG for compile-time overloads; HonestEmpty structurally can't fire; a new variant needs its own
>    review). No metric-definition change in this plan.

**Goal:** Remove the ingestion pollution, resolve the report-dataitem population, and stratify the conflated Unknown
reasons — driving CDO real-`unknown` down honestly (~180 → ~140s, measured not promised) with zero false-`Source`/`Catalog`
(`genuine_wrong` stays 0) and no metric-definition change.

**Tech Stack:** Rust (edition 2024). No new dependency. No `engine::l3`/`engine::l2` import in `src/program/resolve`
(grep-guarded; L2's dataitem seeding is a PRECEDENT to mirror, not import).

**Source of truth:** the two grounding reports (file:line-verified + CDO-sampled) + master `4921746` + the charter (§5, A1).

## Key facts (verified on `4921746`)

**T1 — `.dependencies/` ingestion-scope bug:**
- `walk_al_source` (`src/snapshot/provider.rs:27-42`) skips only `.alpackages`/`.snapshots`/`node_modules` — NOT
  `.dependencies`. The CDO workspace contains `Al/.dependencies/CDO/**` — a stray DECOMPILED reference cache (older,
  differently-numbered object ids, e.g. `Table 6175320` vs live `Table 6175275`) ingested as WORKSPACE tier and resolved
  against the live graph → version-drift noise: 9/25 MemberNotFound sites live there; other buckets may also be polluted;
  the DENOMINATOR (18104) includes its obligations.
- Fix: skip `.dependencies` in the walker (a dot-directory convention skip is defensible: skip ALL dot-directories, or add
  the literal name — decide by what the walker's contract claims). MEASURE the full before/after on CDO: denominator,
  every bucket, `genuine_wrong` (the frozen golden's site keys may reference removed sites — adjudicate; `fresh_missing`
  may DROP). This intentionally changes measured numbers — every delta adjudicated + documented, ratchets re-derived.

**T2 — report-dataitem receivers (the grounding's full design):**
- IR ALREADY carries: `ObjectDecl.report_dataitems: Vec<(String,String)>` (`decl.rs:20-26`, unquoted, flat/object-wide
  scope by design) + `RoutineDecl.dataitem_source_table` (`:161-165`, the innermost enclosing dataitem's table for member
  triggers) + `enclosing_member` (`:166-172`). Lowerer populates both (`lower/mod.rs:318-334`, `:362-414`).
- Design (mirrors L2 `ir_walk.rs:1864-1883` semantics; ZERO new resolution primitives): (i) `DataitemNode{name_lc, name,
  source_table: ObjectRef}` on `ObjectNode` (mirror `page_controls`/`fields`; Report|ReportExtension only); (ii) a
  **Step 2b** dataitem-name lookup in `infer_receiver_type` (`receiver.rs`) strictly AFTER the Step-2 var-lookup miss
  (vars/params/globals SHADOW dataitems — the L2 skip-on-collision rule) → resolve `source_table` via the existing
  fail-closed `resolve_source_table_ref` → `Record{table}`; (iii) thread `dataitem_source_table` into `infer_implicit_rec`
  (the Report/ReportExtension arm currently returns `Record{None}` with a "deferred" comment at `receiver.rs:1654-1662`)
  — covers bare `Rec`/field refs INSIDE a dataitem trigger; (iv) ReportExtension: own dataitems + fall back to the
  extended base Report's list via `extends_target` (the Task-5-of-plan-4 PageExtension `source_table` fallback pattern,
  `receiver.rs:1609-1624`).
- TWO pre-existing defects to fix WITH it (grounding-verified): (a) **the naive dot-substring quote guard** — `receiver.rs:643`
  (+ the mirrored guards at `:718`/`:794` and `full.rs:371`) uses `!receiver_lc.contains('.')`, so a QUOTED identifier with
  an embedded period (`"Sales Cr.Memo Header Filter"` — 5 of CDO's 16 real dataitem names, 10/29 uses) never reaches the
  atomic-token lookups and is mislabeled CompoundReceiver. Fix: quote-aware atomic-token check (a leading-quote token that
  closes at the end is ONE identifier regardless of interior dots) BEFORE the raw dot-scan. This also closes plan-6's
  "dot-quoted names" roadmap note — and it may re-bucket some of the 61 CompoundReceiver sites into resolvable paths.
  (b) **the ReportExtension `modify()` lowerer gap** — `RawKind::ModifyModification` carries `Target` not `Name`
  (`nodes.rs:4431-4455`), so `collect_routines`'s fallthrough (`mod.rs:402`, requires `FieldName::Name`) leaves triggers
  inside `modify(ExistingDataItem)` with `enclosing_member=None` AND `dataitem_source_table=None`. Fix in the lowerer
  (read `Target`), or a resolve-time fallback (look up `enclosing_member` against the merged dataitem list) — prefer the
  lowerer root-fix; note the al-syntax crate boundary (dual consumers: L2 features may shift — run the full suite).
- CDO population: 16 dataitem declarations in 3 report files; ~29 quoted-name receiver uses in Report 6175283 alone; the
  engine-measured "27 dataitem-named" UntrackedReceiver subpopulation. XmlPort/Query: NOT IR-modeled and ZERO on CDO —
  explicitly out of scope (roadmap).

**T3 — reason-SPLIT (diagnostic, count-preserving — the landed `UnknownReason` pattern from the soundness+strat plan):**
- OverloadAmbiguous conflates 4 emission shapes in `resolve_in_object` (`resolver.rs:268-382`): (1) `pre_filter_count == 0`
  — name found, NO overload matches the arity (`:315-320`) → NEW reason `ArityMismatch` (it is NOT ambiguity — nothing
  matched); (2) access-narrowed-to-1 with `pre_filter_count > 1` (`:341-377`) → keep as OverloadAmbiguous or a distinct
  `AccessNarrowedAmbiguous` (decide in review — it IS a candidate-set ambiguity where access can't legally pick); (3)
  `routine_is_collapse_marked` (`:375-377`) → NEW reason `AbiCollapsedOverload` (an ingestion-fidelity admission, not a
  candidate set); (4) the genuine `>1 visible same-arity distinct RoutineNodeIds` (`:380`) → stays `OverloadAmbiguous`
  (the textbook case — verified live: `HttpMgt.DownloadFile(ReadStream, Url)` vs two real 2-arg source overloads).
- MemberNotFound conflates 2 shapes: (A) **object-not-found** (`resolve_object_run` `:1191-1202`; `resolve_member` Object
  arm `:1582-1586` — the receiver OBJECT is absent from the graph) → NEW reason(s): `ObjectNotInGraph`, and where the
  target names an app/page provably outside the declared snapshot (the CTS-CDN case — charter A1 open-world) →
  `UndeclaredExternalTarget`; (B) **member-absent-on-a-resolved-surface** (`resolve_bare` Step-5 `:955/:1105-1106`;
  `resolve_member` post-`resolve_in_object`-None `:1663/:1681`; interface per-implementer `:1733/:1739`) → stays
  `MemberNotFound`, and additionally tag the receiver's TIER in the diagnostic (only source-complete tiers could ever
  prove absence; SymbolOnly never can — local/internal absent from ABI).
- INVARIANTS (the landed pattern): count-preserving (the `unknown` total and every OTHER bucket byte-identical modulo
  T1/T2's real changes — sequence the measurement per-task); `sum(unknownByReason) == unknown`; `Evidence::kind()`
  projection untouched (goldens byte-identical); the aldump/graphify `unknown_reason` strings extend (BC-Brain consumes
  them — additive new keys only, existing keys keep their meaning for the residual population).
- DEFERRED (explicitly, with the grounding data recorded in the CHANGELOG): the outcome-reclassification proper — a new
  `ObligationOutcome` for genuine overload ambiguity (candidate-carrying, non-default-reachable routes — the
  `ConditionalResolved`/`fires_by_default` precedent, NOT Polymorphic which overstates reachability) + any HonestEmpty-like
  treatment for tier-proven member absence. Its own plan + review.

**Metric gates:** metric gate (ceilings 0.00995/180 — will be RE-DERIVED after T1's denominator change + T2's drops, each
dated); audit gate (`genuine_wrong==0`, `FRESH_MISSING_CEILING=3`, `FRESH_WRONG_CEILING=149` — T1 may shift these,
adjudicate); both applicability gates + the preflight; `sum==unknown`.
`CDO_WS="U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud"`, SINGLE tests, FOREGROUND cargo ALWAYS (no background
runs/monitors — they idle agents).

## Round-1 review addenda (BINDING — supersedes conflicting text elsewhere in this plan)

**T1 (both reviewers, Critical):**
- Skip ONLY the literal directory name `.dependencies` — NOT all dot-directories (source could legitimately live in a
  dot-dir; silently vanishing source could produce false `Catalog` via stale resolution). Document the walker contract.
- **Partitioned golden/metric adjudication (the honest T1 acceptance rule):** (1) EVERY removed obligation/site key must
  originate from a `*/.dependencies/*` path — a removed key from any other path is a HARD FAILURE; script the diff, don't
  eyeball it. (2) Retained sites: pre/post compared on IDENTICAL keys — any changed `Source`/`Catalog` edge is manually
  adjudicated; any `genuine_wrong` blocks. (3) New sites: zero (else explain). (4) `fresh_missing`/`fresh_wrong` drops
  caused by removed dep-sites are recorded as `scope_removed`, NEVER credited as modeling improvement. (5) The denominator
  delta must equal the removed-obligation count. (6) Ratchets re-derived only after the scoped-removal manifest is reviewed.

**T2 (both reviewers, Critical/Important):**
- **RequestPage isolation (Critical):** implicit-`Rec` binds ONLY from the current `RoutineDecl`'s own dataitem context
  (`dataitem_source_table`); a `requestpage` trigger (or requestpage/control `modify()`) must NEVER bind a report
  dataitem's table — the lowerer must not thread (or must clear) dataitem context when descending into `requestpage`;
  MANDATORY negative fixture: a requestpage trigger inside a dataitem-bearing report → `Record{None}`/`Unknown`, never the
  dataitem table.
- **modify() lowerer fix is ADDITIVE-ONLY** across the al-syntax boundary (read `Target`; do not alter existing AST/IR
  fields); it must preserve enough context to distinguish DATASET `modify()` from requestpage/control `modify()`; the
  resolve-time fallback (merged own+base dataitem map by `enclosing_member`) fires only for confirmed dataset context;
  fail closed on no-match/duplicate/unresolved-base. Focused lowerer unit test for `ModifyModification.Target` + explicit
  L2 golden/output inspection (full suite alone insufficient).
- **Quote-aware tokenization, exact predicate:** the atomic path requires "no UNQUOTED dot" — for the simple atomic case:
  `starts_with('"') && ends_with('"') && exactly 2 quote chars total`; `"A.B".C` / `"A.B"."C.D"` (an unquoted separator
  present) stay COMPOUND; a quoted atomic receiver that matches no var/dataitem/field → `Unknown` (never guessed).
- **Collision rule fail-closed regardless of AL legality** (the engine ingests partial/decompiled/stale input): dataitem
  name vs same-named report procedure → decline; duplicate dataitem names across own+base maps → decline; comparisons
  case-insensitive + unquoted. Fixtures use explicit `Rec.Field`/`Rec."Field"` inside dataitem triggers (unqualified bare
  fields stay OUT of scope).

**T3 (both reviewers, Critical/Important):**
- **`UndeclaredExternalTarget` is DROPPED** — externality is unprovable from absence (name prefixes/sampling/not-in-graph
  are all disallowed proofs). ALL absent-object shapes emit `ObjectNotInGraph` uniformly. (A metadata-backed external-app
  proof, if ever available, is future work.)
- **The access-narrowed label is `AccessFilteredOverload`** (decided: "narrowed-to-1 then declined" is not genuine visible
  ambiguity; keeps the residual `OverloadAmbiguous` bucket clean). Boundary: pre-access same-arity candidates > 1, access
  filtering narrowed the set, resolver declined rather than select.
- **Tier is a SEPARATE diagnostic field** (e.g. `receiver_tier` alongside the reason), NOT a reason-string split —
  `MemberNotFound` stays one stable key; consumers group by `(reason, tier)`.
- **Per-site bijection invariant:** every pre-T3 Unknown site maps 1:1 to a post-T3 Unknown site with ONLY the
  reason/diagnostic fields changed — `Evidence::kind()`, edge endpoints, route identity, and every non-Unknown bucket
  byte-identical vs the post-T2 baseline (totals alone are insufficient).

**Sequencing discipline:** commit + re-ratchet after EACH task; never land T1's denominator shrink, T2's edge changes,
and T3's reason movement as one combined diff.

## Global Constraints

- Rust edition 2024. `rustfmt <file>` per-file — NEVER `cargo fmt`. Stage only named files — NEVER `git add -A`.
  `CHANGELOG.md` per task. CI gates: `cargo clippy --release --all-features -- -D warnings` (NO `--tests`),
  `cargo fmt --check`, `cargo test --workspace` (NO CDO_WS, green).
- **Soundness cardinal.** Dataitem binding only on a unique name match AFTER var-lookup miss (vars shadow dataitems);
  the proc-shadow discipline holds (a Report has no field surface — but the dataitem lookup must not shadow a same-named
  PROCEDURE-call receiver... a bare `Name.X()` where `Name` is BOTH a dataitem and a parens-less procedure: AL scoping —
  verify + fail closed on collision). The quote-guard fix must not let a genuinely-compound receiver (`A.B` unquoted)
  into the atomic-token paths. All declines → `Unknown`.
- **T1 changes measured reality** — every delta (denominator, buckets, fresh_missing, goldens) adjudicated + documented;
  ratchets re-derived with dated notes; the frozen-golden site keys that referenced removed `.dependencies` sites handled
  per the golden's own update discipline (Rust-owned regen + inspect).
- **T3 is diagnostic-only** — no outcome/evidence-kind change; goldens byte-identical; new reason strings additive.
- **Measure, don't assume**; exhaustive adjudication of every new/changed edge per resolution task; determinism.
- **Out of scope:** the outcome-reclassification proper (deferred, documented); XmlPort/Query dataitems; unquoted bare
  implicit-Rec fields; Sender param-TYPE; protected Variables[].

## Tasks

### Task 1: Fix the `.dependencies/` ingestion scope + full impact adjudication
**Files:** `src/snapshot/provider.rs` (the walker skip), tests + goldens + ratchets.
- [ ] Step 1: failing test — a fixture workspace with a `.dependencies/**/*.al` file: its objects must NOT be ingested
  (and a control: a normal subfolder still is). Decide + document the rule (skip all dot-directories vs the literal name —
  match the walker's documented contract; prefer all-dot-dirs as the convention, note it).
- [ ] Step 2: run — fail. Step 3: implement the skip. Step 4: FULL CDO re-measure — denominator, every bucket,
  `genuine_wrong`/`fresh_missing`/`fresh_wrong`, applicability. ADJUDICATE every delta (expect: MemberNotFound −9ish;
  possible drops in other buckets; denominator shrink; the frozen golden may need Rust-owned regen for removed sites —
  inspect the diff). Re-derive ratchets, dated. Step 5: gates + commit
  `fix(snapshot): exclude .dependencies (stray decompiled caches) from source ingestion (Task 1)`.

### Task 2: Report-dataitem receivers + the dot-quote guard + the modify() lowerer gap
**Files:** `src/program/node_extract.rs` (DataitemNode), `src/program/resolve/receiver.rs` (Step 2b + implicit-Rec
threading + the quote guards), `full.rs` (:371 guard), `crates/al-syntax/src/lower/mod.rs` (the ModifyModification fix),
fixtures `tests/r0-corpus/ws-report-dataitem/` + gates.
- [ ] Step 1: failing fixtures — (a) quoted dataitem receiver in a report trigger (`"Hdr Filter".GetView()`) →
  `Record{table}` + the member resolves; (b) a dataitem name with an EMBEDDED PERIOD (`"Cr.Memo Filter"`) resolves (the
  quote-guard fix); (c) bare `Rec`-field use INSIDE a dataitem trigger types by that dataitem's table (implicit-Rec
  threading); (d) a ReportExtension `modify(BaseDataitem)` trigger gets the dataitem context (the lowerer fix); (e) a
  ReportExtension referencing the BASE report's dataitem name (the extends fallback). NEGATIVES: a local var shadowing a
  dataitem name → the var wins; a dataitem name that is ALSO a report procedure → fail closed (collision); a genuinely
  compound `A.B` unquoted receiver still routes compound (guard precision); non-Report objects unaffected.
- [ ] Step 2: run — fail. Step 3: implement (the grounding's design (i)–(iv) + the two defect fixes; al-syntax lowerer
  change runs the FULL suite — L2 features may shift, inspect). Step 4: CDO gates — UntrackedReceiver should drop ~27;
  the quote-guard fix may re-bucket/resolve some CompoundReceiver sites — adjudicate EVERY new/changed edge exhaustively;
  `genuine_wrong=0`; ratchets, dated. Step 5: gates + commit
  `feat(resolve): report-dataitem receivers (Step 2b + trigger implicit-Rec) + quote-aware token guard + modify() lowering (Task 2)`.

### Task 3: Reason-split — ArityMismatch / AbiCollapsedOverload / ObjectNotInGraph / UndeclaredExternalTarget (+ tier tag)
**Files:** `src/program/resolve/edge.rs` (reasons), `resolver.rs` (the emission sites per the grounding's line map),
aldump/graphify strings, harness invariants.
- [ ] Step 1: failing tests — each new reason emitted at its site (fixtures per shape: an arity-mismatch call; a
  collapse-marked probe; an absent-object member call; an undeclared-external-app target; a genuine >1-same-arity overload
  stays OverloadAmbiguous; a resolved-surface member-miss stays MemberNotFound with the tier tag). INVARIANTS: total +
  other buckets unchanged; `sum==unknown`; goldens byte-identical (the `Evidence::kind()` projection).
- [ ] Step 2: run — fail. Step 3: implement (decide the access-narrowed shape's label in-task with a doc'd rationale).
  Step 4: CDO — count-preserving vs post-T2 baseline; record the NEW stratified breakdown (the deliverable — the honest
  map for the deferred reclassification plan). Step 5: gates + commit
  `feat(resolve): stratify OverloadAmbiguous/MemberNotFound into honest emission-shape reasons, count-preserving (Task 3)`.

### Task 4: Measure + close
- [ ] Full re-measure (all gates); adjudication sign-off (T1 deltas + T2 flips == bucket drops); ratchets at the measured
  floor, dated; CHANGELOG (honest, scoped — incl. the DEFERRED reclassification design rationale + the grounding's
  sampled-bucket data as the next-plan roadmap); charter memory + MEMORY.md. Commit
  `docs(resolve): dataitem receivers + depscope fix + reason-split complete — real-unknown 0.99%→X% (Task 4)`.

## Roadmap — beyond this plan
The outcome-reclassification proper (candidate-carrying routes + a new ObligationOutcome for genuine overload ambiguity —
the ConditionalResolved/fires_by_default precedent; tier-proven member absence) — its own plan + review, now data-driven;
XmlPort/Query dataitem modeling (zero on CDO); unquoted bare implicit-Rec fields; Sender param-TYPE validation; protected
Variables[]; preprocessor symbols for dep parsing (`compilation.rs:26-32` — relevant to any "proven absence" claim).
