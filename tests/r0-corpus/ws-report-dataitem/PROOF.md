# Compiler proof — report-dataitem receivers (dataitem-receivers plan, Task 1)

**Status: SPEC-STATED, NOT COMPILER-RUN.** No working `al`/`alc` compiler is available in this
task's execution environment (see `ws-bare-implicit-rec-field/PROOF.md` and
`ws-record-field-chain/PROOF.md` for the identical caveat, previously recorded). Structural
validity IS verified: every `.al` file in this directory parses with **zero ERROR/MISSING
nodes** under `tree-sitter parse` (`tree-sitter-al/`, the grammar this project owns and
validates against 15,358 real production files). The `cdo_l3_semantic_audit_no_fresh_wrong`
gate (`genuine_wrong == 0`) is the executable regression backstop; this document is the
intended correctness proof artifact, reinforced by the Rust-level test suite in
`tests/program_resolve_harness.rs` (this fixture, loaded end to end via
`resolve_full_program`), unit fixtures in `src/program/resolve/receiver.rs`
(`infer_receiver_type`'s Step 2b and the Report/ReportExtension arm of `infer_implicit_rec`),
and lowerer unit tests in `crates/al-syntax/src/lower/mod.rs`
(`modify_modification_target_becomes_enclosing_member_and_sets_dataset_context` /
`modify_modification_inside_requestpage_never_sets_dataset_context`).

## What this fixture covers

1. **Routine-contextual implicit Rec** (`infer_implicit_rec`'s Report/ReportExtension arm):
   a trigger nested inside `dataitem(Cust; "RD Customer")` types `Rec` by the dataitem's
   source table — `RDBase.Report.al`'s `Cust.OnAfterGetRecord`, `Rec.GetDisplayName()`.
2. **Dataitem-NAME receiver** (`infer_receiver_type`'s new Step 2b): a bare `Cust.Method()`
   reference resolves against the dataitem's table, routine-independent (the dataitem name is
   in scope as a record var across ALL the report's routines) — `RDBase.Report.al`'s
   `TestBareCustName`.
3. **The quote-aware token guard** (`is_atomic_receiver_token`, centralized): a QUOTED
   dataitem name with an EMBEDDED PERIOD resolves — `RDBase.Report.al`'s
   `TestBareDotBearingName`, `"Sales Cr.Memo Header Filter".GetFilters()`, grounded in real
   CDO source (`Report 6175283 "CDO Update Output Profile"`,
   `dataitem("Sales Cr.Memo Header Filter"; "Sales Header")`).
4. **The `modify()` lowerer gap + resolve-time fallback**: `RDExt.ReportExtension.al`'s
   `modify(Cust) { trigger OnAfterGetRecord() .. }` — the lowerer's additive `Target`-field
   read populates `enclosing_member` + `in_dataset_modify_context`, and the resolver's
   confirmed-dataset-context fallback resolves the implicit Rec via the merged own+base
   dataitem map.
5. **ReportExtension base-dataitem fallback**: `RDExt.ReportExtension.al`'s
   `ExtTestBaseDataitemName` — the extension has NO dataitems of its own; a bare
   `Cust.GetDisplayName()` resolves via the extended BASE report's "Cust" dataitem (mirrors
   the PageExtension `SourceTable` inheritance pattern).

## Negatives

- **REQUESTPAGE ISOLATION** (binding, round-1 addendum): `RDBase.Report.al`'s
  `requestpage { trigger OnOpenPage() begin Rec.GetDisplayName(); end; }` — even with a
  dataitem-bearing dataset in the SAME report, a requestpage trigger's implicit Rec must NEVER
  bind a dataitem's table.
- **Var shadows dataitem** (AL scoping): `RDBase.Report.al`'s `TestVarShadowsDataitem` —
  a LOCAL `Cust: Record "RD Sales Header"` (a DIFFERENT table than the dataitem's) must win
  over the same-named "Cust" dataitem.
- **Collision guard** (fail-closed): `RDBase.Report.al`'s `dataitem("RD Collide"; ..)` +
  `procedure "RD Collide"()` — a dataitem name that is ALSO a report procedure name declines
  rather than guess between "the dataitem record" and "a parens-less call to the procedure".
- **Genuinely compound receiver stays compound**: `RDBase.Report.al`'s
  `TestGenuinelyCompoundReceiverStaysUnknown` — an unquoted `A.B` shaped receiver is never
  mis-routed into the atomic dataitem-name lookup.

## Note on the dataitem/procedure name collision fixture

`RDBase.Report.al` declares BOTH `dataitem("RD Collide"; "RD Customer")` AND
`procedure "RD Collide"()` on the same report. Real AL may reject this name collision at
compile time (no working `alc` available to confirm either way — see the status note above);
tree-sitter does not validate cross-declaration semantic uniqueness, so this fixture exercises
OUR OWN fail-closed collision guard directly — the identical convention
`ws-bare-implicit-rec-field/RBFBase.Table.al`'s "Shadowed Field" fixture already established
for `ResolveIndex::table_scope_has_routine`.
