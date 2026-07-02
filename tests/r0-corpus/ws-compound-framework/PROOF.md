# Compiler proof — `this.<rest>` self-scoped stripping (Task 4)

**Status: SPEC-STATED, NOT COMPILER-RUN**, same caveat as
`tests/r0-corpus/ws-friend-app-internal/PROOF.md` / `ws-object-interface-visibility/PROOF.md`: no
AL compiler (`alc`/`ALC_EXE`) was available in this task's execution environment. The rows below
state AL's `this`-keyword semantics as documented by Microsoft's AL language reference rather than
an actual `alc` compile/diagnostic run. The `cdo_l3_semantic_audit_no_fresh_wrong` gate
(`genuine_wrong == 0`) is the regression backstop; this document is the intended CORRECTNESS proof
artifact — reinforced here by an independent, EXHAUSTIVE hand-adjudication against real CDO source
(see `.superpowers/sdd/task-4-report.md` §6), which is the strongest evidence available.

## What this fixture covers

`infer_this_member` (`src/program/resolve/receiver.rs`) resolves a `this.<rest>` compound receiver
by looking up `<rest>` against a SELF-ONLY scope: `object_globals` ONLY, never
`routine.params`/`routine.locals`. This document records the AL semantics that scope choice relies
on.

## AL `this`-keyword semantics (source: Microsoft Learn, "Use the `this` keyword for codeunit
self-reference", `learn.microsoft.com/.../devenv-al-this-keyword`, fetched 2026-07-02)

> **APPLIES TO:** Business Central 2024 release wave 2 and later.

- "The `this` keyword can be used in codeunits in AL as a self-reference, and it allows passing the
  current object as an argument to methods."
- "It improves readability by indicating that a referenced symbol is a member of the object
  itself."
- "The newest version of the System Application has been updated to use the `this` keyword for
  **referencing methods and globals within the same object**."

The load-bearing phrase is the last one: `this.` addresses **methods and globals** of the object —
not locals, not parameters. A routine's locals/parameters live in that routine's own stack frame,
never the object's; they are not "members of the object itself" in the sense the keyword
disambiguates. This is also the entire POINT of the keyword in every language that has it
(C#/JavaScript/Python precedent, cited in the same doc): `this.Foo` explicitly means the
OBJECT-LEVEL `Foo`, deliberately bypassing any local of the same name that would otherwise shadow
it in an ordinary bare reference.

## Scope decision: `object_globals` only, methods excluded too

The doc's phrase covers BOTH "methods" and "globals". This implementation resolves only the
GLOBALS half (`infer_this_member` — a property-form `this.<Global>` reference, typed via the
global's declared type) and deliberately DECLINES the methods half
(`this.<Method>(...)`, a CALL form) rather than attempt it: dispatching a same-object PROCEDURE's
return type needs `resolve_bare`-style routine lookup (own-object/extension-base/implicit-Rec/
builtin precedence, overload-arity resolution, the local-shadowing guard already built for Task 3's
`Func().Method()` case) — a materially larger scope than this task's brief calls for. Per the
brief's own instruction ("If that self-only scope can't be cleanly distinguished from a shadowing
bare lookup, DEFER `this.` stripping entirely... rather than risk a false `Source` on invalid
syntax"), the CALL form stays honestly `Unknown` rather than risk an under-verified resolution.
This is a conservative, DOCUMENTED scope narrowing, not an oversight — see
`this_strip_call_form_declines` (receiver.rs) and `.superpowers/sdd/task-4-report.md` §3/§9.

## Real-world validation (stronger than the spec citation alone)

`Page 6175313 "CDO eDocuments Setup Wizard"` (real CDO source, Continia Document Output) contains,
verbatim:

```al
    var
        ...
        DialogWindow: Dialog;
        ...

    local procedure InitializeDialogWindow()
    begin
        if not GuiAllowed then
            exit;

        this.DialogWindow.Open(this.StatusTxt);
    end;

    local procedure CloseDialogWindow()
    begin
        if not GuiAllowed then
            exit;

        this.DialogWindow.Close();
    end;
```

`DialogWindow: Dialog;` is declared in the object's OWN `var` section (confirmed by reading the
surrounding lines — it sits directly after `trigger OnClosePage()`, alongside other genuine
object-level globals like `MediaRepositoryDone: Record "Media Repository"`), not inside either
procedure shown. This is EXACTLY the `this.<Global>.<Member>()` shape this task's positive fixture
(`TestThisStripDialogWindow` in `tests/r0-corpus/ws-compound-framework/src/CFCaller.Codeunit.al`)
models — the fixture was written to mirror this real, compiling, shipped AL code, not an invented
scenario. Both real call sites (`this.DialogWindow.Open(...)` and `this.DialogWindow.Close()`) were
independently hand-adjudicated as CORRECT resolutions to the `Dialog` catalog's `Open`/`Close`
members during this task's CDO gate pass (`.superpowers/sdd/task-4-report.md` §6, rows 4-5).

## Case-by-case matrix

| Case | Fixture routine | AL rule applied | Rust test(s) |
|---|---|---|---|
| (a) POSITIVE: `this.<Global>` property access | `TestThisStripDialogWindow` | `this.` addresses object globals | `ws_compound_framework_this_strip_dialogwindow_resolves_catalog` (fixture); `this_strip_dialogwindow_resolves_to_dialog` (unit) |
| (b) NEGATIVE: `this.<rest>` must NOT see locals/params | `this_strip_ignores_locals_and_params` (unit only — no dedicated fixture routine, since the fixture already declares `DialogWindow` as a global, so a same-named local there would just prove the SAME resolution, not the exclusion) | locals/params are not "members of the object itself" | `this_strip_ignores_locals_and_params` |
| (c) NEGATIVE: `this.Method(...)` CALL form deferred | `this_strip_call_form_declines` (unit) | typing a procedure's return type is out of this step's scope | `this_strip_call_form_declines` |
