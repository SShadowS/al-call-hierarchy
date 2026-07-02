# Compiler proof — table-field type index + `Rec."Field".X()` record-field chains + EnumType chain base (Task 3)

**Status: SPEC-STATED, NOT COMPILER-RUN** — same caveat as `ws-compound-framework/PROOF.md` and
`ws-chain-tables/PROOF.md`: no AL compiler (`alc`/`ALC_EXE`) was available in this task's execution
environment. The `cdo_l3_semantic_audit_no_fresh_wrong` gate (`genuine_wrong == 0`) is the executable
regression backstop; this document is the intended correctness proof artifact, reinforced by the
Rust-level test suite in `tests/program_resolve_harness.rs` (this fixture, loaded end to end via
`resolve_full_program`) and by hand-built unit fixtures in `src/program/resolve/index.rs` (visibility/
dedupe/duplicate cardinality for `ResolveIndex::field_in_table`) and `src/program/resolve/receiver.rs`
(`infer_compound_member_receiver`'s two new arms).

## What this fixture covers

1. The table-field type index (`FieldNode` on `ObjectNode`, `src/program/node_extract.rs`), populated
   from source `FieldDecl` (`extract_nodes`) and from ABI `AbiTable`/`AbiField` (`abi_ingest::ingest_abi`
   — the ABI-tier path is exercised by a dedicated Rust unit test,
   `abi_table_fields_populate_object_node_field_nodes`, rather than a compiled `.app` fixture here; no
   AL compiler was available to produce one, and the ABI ingestion wiring is fully unit-testable via
   `AbiCache::seed`, matching the precedent `parse_field`'s own Task 2 Subtype-fidelity tests set).
2. `ResolveIndex::field_in_table` — visibility-scoped (base + closure-visible `TableExtension`s only),
   unique-match-or-decline, provenance-deduped before the duplicate check.
3. The new non-method `Member{object, member}` arm in `infer_compound_member_receiver`
   (`src/program/resolve/receiver.rs`): `Rec."Field".X()` / `Rec.Field.X()`.
4. The EnumType-as-chain-base arm (`enum_chain_return_kind`,
   `src/program/resolve/framework_returns.rs`): `Ordinals()`/`Names()` on an Enum VALUE receiver ->
   `Framework(List)`, enabling the multi-level `Rec."Field".Ordinals().Count()` chain.

## Real CDO source grounding (exhaustive grep, not a sample)

Grepped `CDO_WS` (`U:/Git/DO.Support-SlowDOSetup/DocumentOutput/Cloud`) on 2026-07-02:

| Fixture | Real CDO site |
|---|---|
| (a) `Rec."Error Message".CreateInStream(S)` | `Table 6175273 "CDO E-Mail Log"` line 1074: `Rec."Error Message".CreateInStream(IStream);` (guarded by `Rec."Error Message".HasValue()` at line 1072 — a second real Blob-field-chain site, same field) |
| — sibling Blob field | `Table 6175301 "CDO File"` lines 160-161: `Rec."File Blob".CreateOutStream(WriteStream); Clear(Rec."File Blob");` — the field this fixture's naming echoes |
| (b) `Rec."eSeal Service".Ordinals().Count()` | `Page 6175455 "CDO E-Seal Setup Wizard"` line 342: `Rec."eSeal Service".Ordinals().Count() = 1` — field declared `Table 6175329 "CDO E-Seal Setup"` line 13: `field(2; "eSeal Service"; Enum CDOESealService)` |
| (c) TableExtension-folded field | Established pattern (`ResolveIndex::table_extensions_of`, `tests/r0-corpus/ws-bare-extbase-visibility`) generalized to fields — no single CDO grep line, proven structurally by the same folding mechanism routines already use |
| (d) ABI-tier field | `Table 6175273`/`Table 6175301`/`Table 6175329` above are all EmbeddedSource-tier in CDO's own deps (per the CDO harness's own ABI-coverage note in `tests/program_resolve_harness.rs`), so CDO itself does not exercise the ABI-tier field path — covered by the dedicated unit test instead (see item 1 above) |

Both real Blob sites (`Table 6175273`, `Table 6175301`) and the one real multi-level Enum chain
(`Page 6175455`) are exactly the 3 real CDO `CompoundReceiver` sites this task's `unknown` ceiling
delta accounts for (a fourth `HasValue()` call on the SAME `"Error Message"` field at
`Table 6175273` line 1072 is a second edge off the SAME chain-typed receiver, not a new distinct site).

## Negative fixtures — not CDO-grounded, engine-invariant proofs

(e) unknown field name, (f) scalar-typed field, (g) duplicate field name across base+extension, (h) a
Page receiver with a quoted member, and (i) a local variable shadowing a field name are all
**engine-invariant fail-closed proofs** (every AL program, not a specific CDO shape) — grounded in the
task brief's explicit negative list, not a CDO grep line.

## Note on (i)

This fixture uses an UNQUOTED field name (`ErrCode`, `Code[10]`) and an UNQUOTED, BARE local variable of
the SAME name but a DIFFERENT type (`ErrCode: Text[100];`), called bare — `ErrCode.Trim();` — never
`Rec.ErrCode`. The record-field arm this task adds only ever engages via a `Member{object, member}` whose
`object` sub-expression already resolved to `Record{table: Some(..)}` (i.e. is written `Rec.<x>`); a bare
identifier receiver never reaches it at all, by construction (Step 2's variable lookup runs first and owns
bare identifiers entirely — see `infer_receiver_type`'s own inference-order doc). `.Trim()` is a real Text
catalog member, proving the variable wins outright.

An EARLIER draft of this fixture used a QUOTED bare identifier (`"Error Message": Text[100];` /
`"Error Message".Trim();`) to shadow the SAME name as fixture (a)'s field. That surfaced a genuine
PRE-EXISTING, Task-3-independent gap: `infer_receiver_type`'s Step 2 variable lookup
(`src/program/resolve/receiver.rs`) compares the RAW, quote-retaining `receiver_lc` directly against each
`VarDecl.name.to_ascii_lowercase()` (which is always UNQUOTED — the lowerer strips quotes), so a quoted
BARE receiver referencing a variable never matches Step 2 at all — unrelated to record-field chains, out
of this task's scope, and NOT reused here to avoid conflating an unrelated finding with this task's
result. Noted for a future task (bare-quoted-identifier variable lookup is a separate, small fix).
