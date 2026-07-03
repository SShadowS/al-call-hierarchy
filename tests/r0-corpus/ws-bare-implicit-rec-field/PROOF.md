# Compiler proof — bare implicit-Rec quoted-field receivers + routine-shadow guard (Task 4)

**Status: SPEC-STATED, NOT COMPILER-RUN.** An `al`/`alc` binary IS available in this task's execution
environment (`al.exe` 18.0.37.11445, wrapping `alc.exe`), and the platform symbol package
(`Microsoft_System_28.0.48590.0.app`, copied from `CDO_WS`'s own `.alpackages`) was available too — but
every compile attempt against it, even a trivial `codeunit { trigger OnRun() begin end; }` with zero
external references, silently exits non-zero with **no diagnostic on stdout or stderr** (tried both the
`al compile` wrapper and `alc.exe` directly). This is a sandbox/tooling limitation, not a property of the
fixtures below — consistent with the same caveat already recorded in `ws-record-field-chain/PROOF.md`,
`ws-compound-framework/PROOF.md`, and `ws-chain-tables/PROOF.md`. Structural validity IS verified: every
`.al` file in this directory parses with **zero ERROR/MISSING nodes** under `tree-sitter parse`
(`tree-sitter-al/`, the grammar this project owns and validates against 15,358 real production files).
The `cdo_l3_semantic_audit_no_fresh_wrong` gate (`genuine_wrong == 0`) is the executable regression
backstop; this document is the intended correctness proof artifact, reinforced by the Rust-level test
suite in `tests/program_resolve_harness.rs` (this fixture, loaded end to end via `resolve_full_program`)
and by hand-built unit fixtures in `src/program/resolve/index.rs` (`ResolveIndex::table_scope_has_routine`)
and `src/program/resolve/receiver.rs` (`infer_receiver_type`'s Step 2 quote-parity fix and new Step 3a).

## What this fixture covers

1. **Step 2 quote-parity fix** (`infer_receiver_type`, `src/program/resolve/receiver.rs`): the
   var/param/global lookup now unquotes a bare-shaped receiver before comparing against `VarDecl`/`Param`
   names (which are stored already-unquoted by the lowerer's `ident_text`) — a quoted local var now
   resolves as the var, never silently falling through.
2. **Step 3a — bare implicit-Rec quoted-field receiver**: `"Field".X()` with NO `Rec.` prefix, written
   inside a Table/TableExtension's own procedure, means exactly `Rec."Field".X()`. Looks the field up via
   the SAME visibility-scoped `ResolveIndex::field_in_table` surface Task 3's explicit `Rec."Field"` arm
   consults (base + closure-visible `TableExtension`s), gated on `WithState::NoWithProven` (mirrors
   `resolve_bare`'s own Step 3 with-guard) and on the strict `ObjectKind::Table | TableExtension` guard
   (mirrors the bare-call Step-3 precedent).
3. **Round-2 soundness correction — the routine-shadow guard** (`ResolveIndex::table_scope_has_routine`,
   consumed by BOTH Step 3a here AND Task 3's `Rec."Field".X()` compound arm): AL's parens are optional on
   a zero-argument procedure call (`Rec.Insert;` compiles; the Code Cop AA0008 flags the missing parens as
   a STYLE issue, not a compile error), so a bare `Member` AST node — and, more subtly, a bare quoted
   RECEIVER of a further method call — is structurally ambiguous between a field/property access and a
   parens-less call chained through its return value. A same-named routine anywhere in the same
   visibility-scoped table surface must block field-typing.

## Real CDO source grounding

The plan's own measured population (grepped 2026-07-02, `CDO_WS`): **38 real sites** in the
`UntrackedReceiver` bucket — bare `"Field".X()` inside a Table/TableExtension routine, ~35 Blob + 1 Text.
Representative real site: `Table 6175301 "CDO File"` field(20; "File Blob"; Blob), accessed bare
(`"File Blob".CreateOutStream(WriteStream); Clear(Rec."File Blob");`) from within the table's own code —
this fixture's naming and field shape directly echo that real declaration (see also
`ws-record-field-chain/PROOF.md`'s sibling grounding for the same `"File Blob"` field, there accessed via
the explicit `Rec.` form).

## Fixture map

| Letter | File / procedure | Scenario |
|---|---|---|
| (a) | `RBFBase.Table.al` / `TestBareBlobField` | POSITIVE: bare Blob field inside the table's own procedure |
| extra | `RBFBase.Table.al` / `TestBareTextField` | POSITIVE: bare Text field, `.Trim()` |
| (b) | `RBFBaseExt.TableExtension.al` / `TestBareOwnExtField` | POSITIVE: TableExtension's own field, bare |
| (b) | `RBFBaseExt.TableExtension.al` / `TestBareBaseFieldFromExtension` | POSITIVE: base table's field folded into the extension's own scope |
| (c)+(d) | `RBFBase.Table.al` / `TestBareVarShadowsFieldQuoteParity` | NEGATIVE/PRECEDENCE: a quoted local var shadowing a same-named field — var wins (quote-parity fix) |
| (e) | `RBFCaller.Codeunit.al` / `TestBareFieldReceiverNonTableScope` | NEGATIVE: non-Table/TableExtension scope declines, even though the field name is real elsewhere |
| (f) | `RBFBase.Table.al` / `TestBareUnknownField` | NEGATIVE: unknown quoted name |
| round-2 | `RBFBase.Table.al` / `TestBareRoutineShadowsField` | NEGATIVE: a same-named routine ("Shadowed Field") blocks field-typing |

## Note on the routine-shadow fixture

`RBFBase.Table.al` declares BOTH `field(4; "Shadowed Field"; Blob)` AND `procedure "Shadowed Field"()` on
the same table. Real AL may well reject a field/procedure name collision at compile time (no working `alc`
in this environment to confirm either way — see the status note above); tree-sitter does not validate
cross-declaration semantic uniqueness, so this fixture exercises OUR OWN fail-closed routine-shadow guard
directly — the identical convention `ws-record-field-chain/RFCBaseExt.TableExtension.al`'s "Dup Field"
duplicate-field fixture already established for `field_in_table`'s own duplicate-decline logic.

## Note on the quote-parity fixture's dataitem framing

The plan's round-2 addendum describes real CDO sites resolving via this fix as "27 dataitem-named vars"
(`"Sales Header Filter".GetView()`-shaped) — a naming convention echoing Report dataitems, not an actual
Report-dataitem CONSTRUCT. The fresh engine (`src/program`) does not model Report dataitems as record vars
at all (`al_syntax::ir::ObjectDecl.report_dataitems` exists but is consumed only by the legacy L2 engine,
`src/engine/l2/ir_walk.rs` — see the Task 4 report's static var-extraction audit). This fixture therefore
uses a genuine `VarDecl` (`"File Blob": Text[100]` declared as a real local variable), which is exactly
what the quote-parity fix operates on regardless of why a real-world declaration happens to be named that
way.
