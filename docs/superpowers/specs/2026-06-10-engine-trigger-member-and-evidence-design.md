# Engine fix-then-freeze: trigger enclosing-member + opt-in evidence projection — Design

> **Context:** This is a PARITY-CRITICAL change set in the alch-engine (the Rust AL static-analysis
> engine, mid byte-parity migration from the al-sem TypeScript oracle). It unblocks al-perf fusion
> Phase P3.2 (precise field-trigger attribution + causal-chain drilldown), which the al-perf P3 spec's
> Revision 2 deferred behind exactly this engine work. The work is one cohesive change set, proven and
> frozen, BEFORE al-perf P3.2 builds against it. Built on the `engine` branch (LOCAL — never push).
>
> **The migration contract this must not break:** ungated `cargo test` byte-matches the al-sem TS
> oracle goldens; `KNOWN_DIVERGENCES.json` must stay `[]`. Every change here is either on a Rust-only
> projection (safe) or behind a new opt-in flag the parity harness never passes (safe). NO change to
> any default/parity-locked output, NO change to `StableRoutineId` hashing.

## Goal
Expose, parity-safely, two things al-perf needs to make field-trigger fusion precise and causal:
1. **Enclosing member identity** for field/control/action trigger routines — the field/control/action
   name + the originating object — so two fields' `OnValidate` (which today collapse to one
   `StableRoutineId`) can be told apart by a consumer.
2. **The per-finding evidence path** (the call chain to the issue, with routine/operation/loop
   anchors) on `analyze`, behind an opt-in `--with-evidence` flag.

Both surfaced WITHOUT touching `StableRoutineId` (a hash change would rebaseline every golden) and
WITHOUT changing any default/parity-locked output.

## Components (staged E1 → E4)

### E1 — Capture the enclosing member at L3 (model addition; additive, parity-safe)
**Problem (verified):** `l3_workspace.rs` `collect_routine_nodes(decl)` collects only the
`procedure`/`trigger_declaration` nodes, discarding each trigger's parent
`field_declaration`/`page_field`/`action` wrapper; `project_routine_features(decl, routine, …)`
receives the OBJECT decl, not the immediate parent. So `L3Routine` has no enclosing-member data and
can't get it without a localized change.

**Change:**
- Modify `collect_routine_nodes` to return `Vec<(Option<Node>, Node)>` = `(enclosingMemberNode,
  routineNode)` pairs. For a `trigger_declaration` whose parent is a `field_declaration` /
  `page_field` / `action` (the member-bearing wrappers), `enclosingMemberNode` = that parent; for an
  object-level trigger (`OnRun`, `OnOpenPage`) or a `procedure`, it is `None`. Keep the existing
  DFS-prune collection; just retain the parent at the match point.
- Add two fields to `L3Routine` (`l3_workspace.rs:227-321`):
  - `enclosing_member: Option<String>` — the member identifier (field/control/action name,
    `strip_quotes`'d) from the parent node, `None` for procedures/object-level triggers.
  - `originating_object: Option<String>` — the StableObjectId of the object that DECLARES this trigger
    (for a base-table trigger = the table; for a tableextension/pageextension trigger added to a base
    member = the EXTENSION object). This disambiguates the multi-extension collision (two extensions
    adding a trigger to the same base field). Derive from the object decl already in scope at assembly.
- Populate both during the assembly loop (`l3_workspace.rs:741-763`) from the retained parent node +
  the owning object. Purely additive to the struct.

**Parity safety:** `L3Routine` is an internal model struct — NOT golden-serialized directly. The only
serialized surfaces are the inventory (Rust-only, E2) and `analyze` findings (E3, behind the flag).
Adding fields + threading the parent node changes NO existing serialized bytes. **The implementer MUST
run the full `cargo test` parity suite after E1 to confirm zero golden movement** (the risk is an
accidental reordering/retraversal side effect, not the additive fields themselves).

### E2 — Inventory projection fields (Rust-only; safe; schema 1.0.0 → 1.1.0)
`snapshot_full.rs` `build_inventory_doc` (~1063-1076): emit two optional fields per
`routineInventory[]` entry, AFTER `routineName`, BEFORE `stableRoutineId`:
- `enclosingMember` (only when `Some`),
- `originatingObject` (only when `Some`).
Bump `INVENTORY_SCHEMA_VERSION` `"1.0.0"` → `"1.1.0"` (minor; additive optional fields). The entry
sort stays `locale_compare(stableRoutineId)`; since two field triggers share a stableRoutineId, add
`enclosingMember` as a SECONDARY sort key so duplicate-id rows have a content-stable order (the P3
review's R3-8/tie-break point). The `routine-inventory` projection is Rust-only (verified: al-sem TS
never emitted it; not in the parity harness) → no TS change, no golden rebaseline, no
KNOWN_DIVERGENCES impact. Update the Rust-internal `tests/cli_p1_inventory.rs` to assert the new
fields + the 1.1.0 version + the two-field fixture emitting distinct `enclosingMember` on rows sharing
a `stableRoutineId`.

### E3 — `analyze --with-evidence` opt-in (parity-safe by construction)
**The flag:** add `#[arg(long = "with-evidence", default_value_t = false)] pub with_evidence: bool` to
`AnalyzeCli` (`alsem.rs`), thread it through `AnalyzeArgs` → `JsonFormatInputs` → `build_analyze_json`
(mirror how `deterministic`/`format` thread). When FALSE (the default, and ALWAYS in the parity
harness), output is byte-identical to today.

**Under `--with-evidence` only**, augment each finding in the `analyze --format json` payload with:
- `evidencePath`: the `StableEvidenceStep[]` (routineId in `:`-form via `map_routine_id`, sourceAnchor,
  note, optional operationId/callsiteId/loopId) — threaded from the internal `StableFinding.evidence_path`
  (which already exists, `finding.rs:177-229`) through the gate `FindingSummary` (add
  `evidence_path: Option<Vec<…>>`, populated only when the flag is set) into `format_json.rs`'s
  `finding_summary_to_value` (emit only when `Some`).
- On each finding's `primaryLocation` (and/or each evidenceStep's routine): `enclosingMember?` +
  `originatingObject?` — the E1 fields, looked up by the finding's `enclosingRoutineId` via a
  `routineId → (enclosingMember, originatingObject)` map built from the resolved routines. This is the
  finding-side discriminator that lets al-perf attribute a field-trigger finding to the RIGHT field
  (the P3 review's R3-3 MUST-FIX — without it, P3a makes the method `matched` while findings stay
  field-ambiguous).

**Schema version:** the default (no-flag) `analyze` envelope `schemaVersion` STAYS `"1.0.0"` (parity).
DECISION FOR REVIEW: under `--with-evidence`, either (a) keep `"1.0.0"` and rely on additive optional
fields (al-perf's `majorMatches` accepts it), or (b) emit `"1.1.0"` to signal the augmented shape.
Recommend (b) — explicit superset signal — but ONLY under the flag, so the default/parity output is
untouched. al-perf pins `EXPECTED_ANALYZE_SCHEMA_VERSION` and accepts minor bumps via `majorMatches`.

**Parity safety (verified):** `tests/cli_a_json_differential.rs` byte-compares `--deterministic
--format json` (NO `--with-evidence`) against al-sem goldens. Since every new field is gated on the
flag (and `skip_serializing_if`/conditional when absent), the default output is byte-identical → the
40 golden tests stay green, KNOWN_DIVERGENCES stays `[]`.

### E4 — Prove + freeze
- Full `cargo test` green (the parity differential suites unchanged — default outputs byte-identical;
  KNOWN_DIVERGENCES `[]`).
- New Rust tests: the two-field-`OnValidate` fixture → inventory emits distinct `enclosingMember` per
  field on rows sharing a `stableRoutineId` (E2); `analyze --with-evidence` emits `evidencePath` +
  the finding-side `enclosingMember`/`originatingObject`, while `analyze` (no flag) is byte-identical
  to its golden (E3); a multi-extension fixture → distinct `originatingObject`.
- Bump the relevant CLI cache/version surfaces if required by the engine's version-tuple tests
  (the implementer checks `CACHE_VERSIONS`/version tests for the inventory + analyze schema bumps).
- Tag this as the frozen schema al-perf P3.2 consumes.

## Data flow
L0 parse → **E1**: L3 assembly captures `(enclosingMember, originatingObject)` per trigger routine on
`L3Routine` → **E2**: inventory projection emits them (Rust-only, 1.1.0) → **E3**: `analyze
--with-evidence` surfaces `evidencePath` (from the internal StableFinding) + the finding-side
member/object discriminator (from the L3 fields, looked up by enclosingRoutineId) → al-perf P3.2
consumes the frozen schema.

## Error handling / non-invasiveness
- E1: additive struct fields, `None` for non-member routines; no behavior change.
- E2: Rust-only projection; minor additive schema bump; no parity surface.
- E3: opt-in flag; default output byte-identical (the parity contract); fields conditional.
- No `StableRoutineId` hash change anywhere (avoids a migration-wide golden rebaseline).
- The engine never throws — missing parent/member → `None`, surfaced honestly as absent fields.

## Risks for the external (adversarial) review to stress
1. **E1 parity side effects (the #1 risk):** does changing `collect_routine_nodes` to return
   `(parent, routine)` pairs, or threading the parent node, perturb ANY existing traversal order,
   routine collection set, or serialized field (analyze findings, snapshot, digest, events)? It MUST be
   provably additive — the same routines, same order, same existing bytes. How to verify beyond
   "cargo test green" (e.g. assert the routine set/order is identical pre/post)?
2. **enclosingMember derivation correctness:** for the parent node kinds (`field_declaration` on
   tables, `page_field`/`action` on pages, table/page EXTENSION variants), is the member identifier
   always the expected child, and is `strip_quotes` the right normalization? Are there trigger kinds
   whose parent is NOT a member wrapper but also not object-level (e.g. a control-add-in, a
   `requestpage`, a report dataitem trigger) where `enclosingMember` should be `None` vs a real name?
   Does the choice match what the al-perf profile's `"<member> - <trigger>"` name carries?
3. **originatingObject semantics:** for a tableextension/pageextension trigger added to a BASE member,
   is `originatingObject` the extension or the base object — and which does al-perf need to
   disambiguate the multi-extension collision (gemini's R3-8 case)? Is the StableObjectId of the
   declaring object available at assembly?
4. **E3 finding-side discriminator soundness:** the `routineId → (enclosingMember, originatingObject)`
   lookup by `enclosingRoutineId` — can it mis-resolve (a finding whose enclosing routine is a
   collapsed field trigger maps to ONE routineId shared by both fields → the lookup returns ONE
   member, not the finding's actual field)? THIS IS THE CRUX: does surfacing `enclosingMember` on the
   finding actually disambiguate, or does the same `StableRoutineId`-collapse that defeats P3a also
   defeat this (the finding's `enclosingRoutineId` is itself the collapsed id)? If so, the
   discriminator must come from the finding's SOURCE POSITION / the L3Routine that owns the finding's
   primaryLocation line, NOT from the collapsed routineId. Verify which is sound.
5. **Schema-version + cache-tuple:** does bumping INVENTORY_SCHEMA_VERSION (and optionally the
   --with-evidence analyze schema) require a cache-version/version-tuple test update, and does it stay
   parity-safe?
6. **Determinism:** the secondary sort key (E2), the evidencePath ordering (E3) — byte-stable under
   `--deterministic`?

## Non-goals
al-perf P3.2/P3a/P3b themselves (this is only the engine substrate). Changing `StableRoutineId`
hashing (explicitly avoided). The `digest` effect projection (al-perf P3b uses `--with-evidence`
evidencePath; digest stays as-is). Any default/parity-locked output change.

## Self-review notes
- **E1 is the foundation and the highest parity risk** (a traversal change in the migration core);
  E2 is Rust-only-safe; E3 is opt-in-safe by construction.
- **Parity is the spine:** no default output moves, no StableRoutineId change, KNOWN_DIVERGENCES `[]`.
- **The crux (risk #4):** the finding-side discriminator must be sound against the very
  StableRoutineId-collapse that motivates the work — surface the member from source position, not the
  collapsed id, if the collapsed id can't distinguish the fields.
- **Freeze before P3.2:** prove the schema, then al-perf builds against it.

---

## Revision 2 — folded from the three-reviewer adversarial pass (2× opus + gemini-3.1-pro)

The design body above is SUPERSEDED where it conflicts here. All three reviewers verified against
actual code; convergent findings below. Parity safety of E1/E2/E3 was CONFIRMED (no golden byte moves —
`L3Routine` is not serde-serialized, routine set/order is provably unchanged, inventory is Rust-only,
`--with-evidence` is never passed by the harness). The blockers are soundness/correctness of the
discriminator and the cross-system join, plus mechanical compile-gates. Implement to THIS revision.

### RE-1 — The finding-side discriminator is POSITION-BASED, never id-based (MUST) [opus×2]
The internal `L3Routine.id` collapses identically to `StableRoutineId` — `compute_routine_id`
(`scope.rs:560-588`) keys on `(app, objType, objNum, kind, name, params, ret)`, all identical for two
fields' `OnValidate`; the gate routine map (`projection.rs:55-58`, `routines.iter().map(|r|
(r.id, r))` into a HashMap) is last-writer-wins. So an `enclosingRoutineId`-keyed member lookup returns
ONE field's member for BOTH findings (~50% wrong). **Strike the id-based lookup.** The sound mechanism:
match the finding's `primary_location` (same `source_unit_id`) to the member-bearing wrapper node whose
range CONTAINS it, smallest containing range, and use THAT member's name. Two field triggers are two
distinct `trigger_declaration` nodes at disjoint ranges → unique containment. Add a two-field-`OnValidate`
fixture test asserting EACH FINDING gets ITS OWN field name (per-finding, NOT per-inventory-row — the
inventory test would pass trivially while the finding map is wrong).

### RE-2 — Match the MEMBER-WRAPPER range, not just the trigger-body range (MUST) [gemini]
RE-1's containment must use the member WRAPPER node (`field_declaration`/`page_field`/`action_declaration`/
`report_dataitem` — the node spanning the WHOLE member incl. its properties), not only the nested
`trigger_declaration` bounds. A finding reported on the member declaration itself (outside the trigger
body) would otherwise fall back to the object scope and lose its member. So E1 must capture, per
member-trigger routine, BOTH the `enclosing_member` string AND the member-wrapper source RANGE (start/end
line/col + source_unit_id); the finding-side lookup matches against that wrapper range. (For the al-perf
fusion use case the relevant findings sit inside trigger bodies, but the wrapper range is the robust
boundary and costs nothing extra.)

### RE-3 — Member-name derivation: `child_by_field_name("name")`, not "first child"; widen the parent set (MUST) [opus-2]
"First identifier/quoted_identifier child" is WRONG for `field_declaration` — its first named child is
the integer field NUMBER (grammar `field('id', integer); field('name', ...)`). Derive the member via
`parent.child_by_field_name("name")` then `strip_quotes`. This works uniformly for the page/action/
dataitem wrappers too. Fixes also: the page-action node kind is **`action_declaration`** (not `action`);
include **`report_dataitem`** and **`query_dataitem`** as member-bearing parents (a report dataitem's
name appears in al-perf profile frames, e.g. `"Customer - OnAfterGetRecord"`). RULE: treat ANY immediate
parent that is not the object decl and exposes a `name` field as member-bearing; `None` otherwise (true
object-level triggers `OnRun`/`OnOpenPage`, and wrappers without a `name`). `actionref_declaration` uses
`promoted_name` not `name` and has no trigger body → `None`.

### RE-4 — Canonicalize the member string for the cross-system 3-way join (MUST) [gemini]
al-perf joins THREE member strings: the CPU-profile frame (`"Sell-to Customer No. - OnValidate"`, runtime-
emitted, al-perf cannot change), the E2 inventory `enclosingMember`, and the E3 finding `enclosingMember`.
The engine's two surfaces agree by construction (same AST string), but they must also match the PROFILE
frame. Two corpus-invisible breakers:
- **Escaped quotes:** an AL identifier `"Sell-to ""Custom"" No."` — `strip_quotes` only trims boundary
  quotes; the engine must EMIT THE LOGICAL NAME (unescape internal `""` → `"`) so it matches the
  profiler's display form. Specify: `enclosingMember` is the unescaped logical identifier.
- **Case:** AL is case-insensitive; the profiler may normalize case differently from the source text.
  CONTRACT: the engine emits the member as-written (unescaped); **al-perf MUST join case-insensitively**
  (document this in the al-perf P3.2 spec). Do NOT lower-case in the engine (that would diverge the
  inventory's human-facing field from source); the case-insensitive comparison lives on the join side.

### RE-5 — `originatingObject` is honest metadata but likely UNJOINABLE from the profile; document the limit (MUST) [gemini]
The multi-extension collision (two `tableextension`s adding `OnValidate` to the same base field) needs the
declaring object to disambiguate — but the AL CPU-profile frame carries only `"<member> - OnValidate"`
with NO extension identity, so al-perf has nothing to join `originatingObject` against. Keep
`originatingObject` (= the StableObjectId of the object decl being assembled — the extension; available at
`l3_workspace.rs:643`) as honest metadata, but DO NOT claim it resolves the multi-extension collision.
Document in BOTH specs: the different-fields-same-trigger case is resolved by `enclosingMember` (the
high-value, common fix); the two-extensions-same-base-field-same-member case remains UNRESOLVABLE from
the profile and stays honestly `ambiguous` in al-perf. (E1 confirmed NECESSARY — re-parsing the AST at
format time is a perf anti-pattern; collect the cheap string at L3 assembly.)

### RE-6 — E2 inventory sort: three-key, case-insensitive on member (MUST) [gemini, opus-1]
`build_inventory_doc` sort becomes: `locale_compare(stableRoutineId)` →
`case_insensitive_compare(enclosingMember)` → `locale_compare(originatingObject)`. Without the
case-insensitive secondary key, duplicate-stableRoutineId row order would depend on developer casing
(nondeterministic across equivalent code). Update `cli_p1_inventory.rs` for the 1.1.0 version + the new
fields + the deterministic tie-row order.

### RE-7 — Mechanical compile-gates + lowest-blast-radius wiring (MUST) [opus-1, opus-2]
- **Touch only the `l3_workspace.rs` copy of `collect_routine_nodes`** (4 other private copies exist in
  L2: `l2/mod.rs:468`, `l2/l2_workspace.rs:441`, `l2/operation_order.rs:860`, `l2/control_context.rs:696`
  — editing those would ripple into L2 goldens). Capture the parent via `node.parent()` at the existing
  DFS match point WITHOUT restructuring the stack (preserves push order).
- **Prefer attaching evidencePath POST-projection** in `run.rs` (~364) where `paired: (FindingSummary,
  &Finding)` already carries the raw `Finding.evidence_path` — rather than threading the `--with-evidence`
  flag into `project_finding` and adding a field to `FindingSummary` (which is a struct-literal at ~7
  sites: `projection.rs:91`, `filter.rs:99`, `baseline.rs:117`, `format_pr_summary.rs:288`,
  `format_terminal.rs:490`, `inline_suppression.rs:305`/`:346`). If a `FindingSummary` field is
  unavoidable, it MUST be `Option` defaulting `None` at every literal + `skip_serializing_if`. NOTE: the
  evidencePath surfaced on `analyze` comes from the internal `Finding.evidence_path` (`finding.rs:96`),
  NOT the R4-projection `StableFinding` — the spec body's citation of `StableFinding` is corrected here.
- `AnalyzeArgs`/`AnalyzeCli` gain `with_evidence: bool` → `with_evidence: false` at every test literal
  (e.g. `cli_a_json_differential.rs:129,478`); the harness leaves it false → default output byte-identical.

### RE-8 — Schema/cache verification (CONFIRMED, no tuple bump) [opus-1]
The cache version tuple (`cli_c_cache_differential.rs:347-354`: `analyzer, depCache, devFingerprint,
grammar, resourcePolicy, summarySchema, symbolReader`) does NOT include `INVENTORY_SCHEMA_VERSION` or the
analyze envelope `schemaVersion`. So: bump `INVENTORY_SCHEMA_VERSION` 1.0.0→1.1.0 (update only
`cli_p1_inventory.rs`); keep the no-flag analyze `schemaVersion` at `"1.0.0"` (the harness asserts it at
`cli_a_json_differential.rs:394`); emit `"1.1.0"` ONLY under `--with-evidence`. No cache-tuple change.

### RE-9 — Verify zero golden movement beyond "cargo test green" (SHOULD) [opus-1]
Add an order/set invariant: assert `workspace.routines` (count + the `(id, source_anchor.start)`
sequence) is identical pre/post the E1 `collect_routine_nodes` change on a multi-trigger fixture — the
real risk is an accidental retraversal, which a green differential might not localize.

### RE-10 — Empirical validation from real .alcpuprofile data (corrects RE-3/RE-5) [user-provided profiles]
Verified against real BC profiles at `U:\Git\al-perf\exampledata\` (sampling + instrumentation). The
profile node shape: `callFrame.{functionName, scriptId, url, lineNumber}` + `declaringApplication.{appName,
appPublisher, appVersion, appId}`. Findings:
- **`enclosingMember` join CONFIRMED for fields.** Field-trigger frames are exactly `"<member> - <Trigger>"`
  with member = the UNQUOTED display name (e.g. `"Sell-to Customer No. - OnValidate"`, `"Ship-to
  Country/Region Code - OnValidate"`, `"Line Discount % - OnValidate"`). Matches the AL field name →
  the engine's `strip_quotes`'d `enclosingMember` byte-matches. No escaped-quote cases in the corpus
  (the RE-4 unescape stays as correct defensive handling). Separator is `" - "`; procedures with an
  "OnValidate" SUBSTRING (`NoOnAfterValidateCRE`, `Sales Line_OnValidate…`) have NO `" - "` → reliably
  distinguished from field triggers.
- **RE-5 CORRECTED — `originatingObject` IS joinable (the review's pessimism was wrong).** An
  extension-declared field trigger frame carries `scriptId:"PageExtension_50022"` (the EXTENSION object,
  not the base) + `declaringApplication.appId:"0c88…"` (the extension's app). So the profile DOES carry
  the declaring-object identity. al-perf can match `originatingObject` (the declaring object's
  StableObjectId = appGuid:objectType:objectNumber) against `(declaringApplication.appId, scriptId→
  objectType/number)`. The two-extensions case IS resolvable. KEEP `originatingObject` as a real
  disambiguator. (Frames also carry `lineNumber` — a secondary position signal if ever needed.)
- **RE-3 REFINED — ACTION triggers carry the CAPTION with `&`, not the action name.** Frames like
  `"Re&lease - OnAction"` / `"Re&open - OnAction"` use the action Caption (with the `&` accelerator),
  whereas the engine's `enclosingMember` (from `child_by_field_name("name")`) is the action NAME
  (`Release`). So action-trigger `enclosingMember` will NOT byte-match the profile. DOCUMENT in the
  al-perf P3.2 spec: al-perf must strip `&` from action frames before joining, and treat action-trigger
  attribution as best-effort (field triggers — the dominant fusion case — join cleanly). The engine
  emits the action NAME (correct, source-faithful); the accelerator/caption mismatch is a join-side
  normalization al-perf owns. (Page/field/dataitem members are unaffected.)

### RE-11 — Same-trigger-name sibling-field dedup is PRE-EXISTING; the freeze strictly improves on it [E3 empirical]
Empirically confirmed during E3: when two fields of the same object carry the SAME trigger
(identical trigger name + kind + signature — e.g. two `OnValidate` triggers) with the same
finding-triggering pattern, the engine emits ONE finding for the collapsed group, not one per
field. Cause: the internal routine id is position-INDEPENDENT (`compute_routine_id` ignores the
enclosing member — the RE-1 collapse), so both field triggers map to the SAME internal RoutineId,
and a detector's first-wins-by-finding-id dedup (e.g. `d1.rs`) keeps only the first. This is
PRE-EXISTING engine behavior — NOT introduced by this change set.

The freeze IMPROVES on the prior state without regressing anything: the SURVIVING finding goes
from member-AMBIGUOUS (pre-freeze: no member, or a last-writer-wins routine-map member that could
name the WRONG field) to PRECISELY attributed via the E3 position-derived discriminator (the member
of the wrapper that actually CONTAINS the surviving finding's projected primaryLocation). It does
NOT separately emit the sibling field's same-pattern finding.

Consequence for al-perf P3.2: expect PRECISE attribution for the surviving finding of a
same-trigger-name collision group — NOT a second finding per colliding sibling field. This is
acceptable (strictly better than pre-freeze; nothing regresses). Fields with DISTINCT trigger kinds
(e.g. one `OnValidate` + one `OnLookup`) do NOT collapse and yield one precisely-attributed finding
each (the E3 `cli_a_with_evidence` two-field test exercises this distinct-kind case).

### RE testing additions
- Two-field-`OnValidate` fixture: inventory emits distinct `enclosingMember` per row sharing a
  `stableRoutineId` (E2); `analyze --with-evidence` attaches `evidencePath` + per-FINDING distinct
  `enclosingMember` (position/wrapper-range derived — RE-1/RE-2); default `analyze` byte-identical to
  golden (E3).
- Escaped-quote + mixed-case field-name fixture: `enclosingMember` is the unescaped logical name (RE-4).
- `report_dataitem` `OnAfterGetRecord` fixture: `enclosingMember` = the dataitem name (RE-3).
- Multi-extension fixture: distinct `originatingObject` per extension (metadata), with a doc note that
  it's profile-unjoinable (RE-5).
- Routine set/order invariant (RE-9); full parity differential green + KNOWN_DIVERGENCES `[]` (E4).
