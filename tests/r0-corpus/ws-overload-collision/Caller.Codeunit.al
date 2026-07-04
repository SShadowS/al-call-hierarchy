// beyond-1B.3b Task 2 fixture: `CallAmbiguous` invokes the colliding
// same-arity overload set on `Ambiguous.Codeunit.al` (member-Object dispatch)
// — must resolve honest ambiguous/Unknown, never a guessed `Source` route.
// `CallControl` invokes the single-overload control target on
// `Control.Codeunit.al` — must still resolve cleanly to `Source`.
//
// argtype-dispatch-and-page-catalog plan, Task 2: `CallAmbiguous`'s Integer
// LITERAL argument is now compiler-proven evidence (an Integer literal
// cannot bind `Code[20]`) — REBASELINED to a confident pick of the Integer
// overload (see `ws_overload_collision_ambiguous_call_becomes_resolved_to_
// the_integer_overload`).
//
// pageext-merge-and-final-residual plan, Task 3 (a SECOND rebaseline of the
// SAME fixture): `CallAmbiguousUntyped`'s `GetValue()` call-result argument
// — deferred/untyped through Task 2 — is now typed by `arg_dispatch::
// type_one_arg`'s new `Call` arm (`GetValue(): Integer`, unshadowed,
// unambiguous). `Integer` exact-matches `Resolve(X: Integer)` and PROVABLY
// eliminates `Resolve(X: Code[20])` — REBASELINED to the SAME confident
// pick `CallAmbiguous`'s literal already gets (see
// `ws_overload_collision_untyped_arg_call_picks_integer_overload_via_call_
// result_arg`).
codeunit 50962 "Overload Collision Caller"
{
    procedure CallAmbiguous()
    var
        Target: Codeunit "Overload Collision Target";
    begin
        Target.Resolve(5);
    end;

    procedure CallAmbiguousUntyped()
    var
        Target: Codeunit "Overload Collision Target";
    begin
        Target.Resolve(GetValue());
    end;

    procedure CallControl()
    var
        Ctrl: Codeunit "Overload Collision Control";
    begin
        Ctrl.Solo(5);
    end;

    local procedure GetValue(): Integer
    begin
        exit(5);
    end;
}
