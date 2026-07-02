// beyond-1B.3b Task 3 fixtures — `Func().Method()` compound-receiver
// resolution (see `infer_call_result_receiver`, `src/program/resolve/
// receiver.rs`). One PROCEDURE per scenario so `edges_for_object_routine`
// can isolate each call obligation cleanly. Own-object prefix procedures
// live at the top; the routines below exercise them (plus a few standalone
// negatives) one at a time.
codeunit 51003 "CallResultCaller"
{
    // ---- Own procedures used as compound-receiver prefixes ------------------

    // (a) POSITIVE prefix: unique arity-0, `Record` return.
    procedure GetCustomer(): Record "CR Customer"
    begin
    end;

    // Codeunit-return-shape POSITIVE prefix: unique arity-0, `Codeunit` return.
    procedure GetHelper(): Codeunit "CR Helper"
    begin
    end;

    // (g) Interface-return POSITIVE/behavioral prefix: unique arity-0.
    procedure GetIFoo(): Interface ICRFoo
    begin
    end;

    // (b) NEGATIVE prefix pair: two overloads, DIFFERENT arities (0 and 1)
    // AND different return types. `TestOverloadArityMismatch` below calls
    // with an arg count matching NEITHER — the wrong-overload guard.
    procedure GetX(): Codeunit "CR Helper"
    begin
    end;

    procedure GetX(A: Text): Record "CR Customer"
    begin
    end;

    // (c) NEGATIVE prefix: scalar (`Integer`) return.
    procedure GetCount(): Integer
    begin
    end;

    // (d2) NEGATIVE prefix: single declared overload, arity 1.
    // `TestArityMismatchSingle` below calls it with arity 0.
    procedure GetSingle(X: Text): Record "CR Customer"
    begin
    end;

    // (f) NEGATIVE prefix: return type "CRHelperShared" is a Codeunit
    // declared in BOTH the "CRLibA" and "CRLibB" dependencies — genuinely
    // cross-app-ambiguous (this workspace declares no "CRHelperShared" of
    // its own).
    procedure GetH(): Codeunit CRHelperShared
    begin
    end;

    // ---- Test routines — one call obligation each ---------------------------

    // (a) POSITIVE: `GetCustomer()` (own, unique arity-0, `Record "CR
    // Customer"` return) types the receiver as `Record{table: Some(CRCustomer)}`;
    // `Name` is a non-builtin Customer procedure — must resolve `Source`,
    // exact target id.
    procedure TestRecordReturn()
    begin
        GetCustomer().Name();
    end;

    // Codeunit-return shape POSITIVE: `GetHelper()` types the receiver as
    // `Object{Codeunit, "CR Helper"}`; `DoWork` must resolve `Source`.
    procedure TestCodeunitReturn()
    begin
        GetHelper().DoWork();
    end;

    // (g) Interface-return POSITIVE/behavioral: `GetIFoo()` types the
    // receiver as `Interface{"icrfoo"}` — Phase B fans out to `ICRFoo`'s sole
    // implementer (`CR Foo Impl`), never a concrete guess.
    procedure TestInterfaceReturn()
    begin
        GetIFoo().Bar();
    end;

    // (b) NEGATIVE — wrong-overload guard: `GetX` is overloaded (arity 0 and
    // arity 1, DIFFERENT return types); calling with 2 args matches NEITHER
    // declared overload — `resolve_bare`'s Step 1 (own object) must decline
    // (`OverloadAmbiguous`-shaped: zero arity-matched candidates), never fall
    // back to either overload's return type.
    procedure TestOverloadArityMismatch()
    begin
        GetX(1, 2).Bar();
    end;

    // (c) NEGATIVE: `GetCount(): Integer` — a scalar return has nothing to
    // dispatch a member call on.
    procedure TestScalarReturn()
    begin
        GetCount().Bar();
    end;

    // (d1) NEGATIVE: prefix name not declared anywhere in this object (or any
    // reachable extension base/implicit-Rec/builtin).
    procedure TestAbsentPrefix()
    begin
        Nonexistent().Bar();
    end;

    // (d2) NEGATIVE: `GetSingle` is declared ONLY at arity 1; called here
    // with arity 0 — arity mismatch against the sole overload.
    procedure TestArityMismatchSingle()
    begin
        GetSingle().Bar();
    end;

    // Local-var-shadow NEGATIVE (round-2 gemini critical): a local `Integer`
    // named identically to this object's OWN `GetCustomer` procedure (used by
    // fixture a, above) SHADOWS it in AL. `resolve_bare` cannot see locals —
    // the shadowing guard must fire BEFORE ever calling `resolve_bare`, even
    // though `GetCustomer` would otherwise resolve cleanly (proving the guard
    // is load-bearing, not vacuous).
    procedure TestLocalVarShadow()
    var
        GetCustomer: Integer;
    begin
        GetCustomer().Bar();
    end;

    // (h1) DEFERRED-shape guard NEGATIVE: `Obj.DoWork().Bar()` — the receiver
    // of `.Bar()` is `Obj.DoWork()`, whose `function` is a MEMBER expression
    // (`Obj.DoWork`), not a bare identifier — the cross-object-chain shape is
    // deliberately deferred (Task 4), so this stays `Unknown`.
    procedure TestCrossObjectChain()
    var
        Obj: Codeunit "CR Helper";
    begin
        Obj.DoWork().Bar();
    end;

    // (h2) DEFERRED-shape guard NEGATIVE: a prefix call with a string-literal
    // argument containing a dot — proves the AST-based (not text/`receiver_
    // text`-based) receiver inspection is never confused by it. `Foo` is not
    // declared anywhere, so this stays `Unknown` regardless.
    procedure TestStringLiteralArg()
    begin
        Foo('a.b').Bar();
    end;

    // (f) NEGATIVE: `GetH()`'s return type "CRHelperShared" is cross-app
    // ambiguous (see the prefix declaration above) — `parsed_type_to_receiver`
    // inherits the fail-closed `resolve_object_ref` decline, never guessing
    // either dependency's Codeunit.
    procedure TestCrossAppAmbiguous()
    begin
        GetH().Bar();
    end;
}
