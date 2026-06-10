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
