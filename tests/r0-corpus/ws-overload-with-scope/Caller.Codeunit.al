// Task 2 review fix (Finding 1) fixture: the dormant wrong-pick vector —
// `arg_dispatch`'s bare-identifier arg typing never used to consult
// `WithState` (dormant on CDO, which has zero `with` blocks). `SomeField`
// is declared BOTH as a table field on "WS With Scope Table" (Decimal) AND
// as a global variable on this codeunit (Text). Inside `with Rec do`, AL
// rebinds the bare identifier `SomeField` to the WITH receiver's field —
// the caller-scope-EXACT lookup `arg_dispatch::type_one_arg` uses
// (params -> locals -> globals) structurally CANNOT see that rebinding, so
// without the with-scope gate it would type `SomeField` as the GLOBAL's
// declared type (Text) and fail-closed-PICK the `Foo(X: Text)` overload —
// a WRONG pick, since the compiler's actual bound type is the field's
// Decimal. `CallInsideWith` proves the gate degrades this call to NO pick
// (stays AmbiguousResolved); `CallOutsideWith` is the control proving the
// SAME call, unshadowed by any `with`, still confidently picks (the Text
// global exactly matches `Foo(X: Text)`, and Text/Decimal are cross-family
// — a proven, not merely undecided, elimination of `Foo(X: Decimal)`).
codeunit 50962 "WS Overload With Scope Caller"
{
    var
        SomeField: Text;

    procedure CallInsideWith()
    var
        Rec: Record "WS With Scope Table";
        Target: Codeunit "WS Overload Target";
    begin
        with Rec do begin
            Target.Foo(SomeField);
        end;
    end;

    procedure CallOutsideWith()
    var
        Target: Codeunit "WS Overload Target";
    begin
        Target.Foo(SomeField);
    end;
}
