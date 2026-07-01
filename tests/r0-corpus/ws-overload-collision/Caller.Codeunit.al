// beyond-1B.3b Task 2 fixture: `CallAmbiguous` invokes the colliding
// same-arity overload set on `Ambiguous.Codeunit.al` (member-Object dispatch)
// — must resolve honest ambiguous/Unknown, never a guessed `Source` route.
// `CallControl` invokes the single-overload control target on
// `Control.Codeunit.al` — must still resolve cleanly to `Source`.
codeunit 50962 "Overload Collision Caller"
{
    procedure CallAmbiguous()
    var
        Target: Codeunit "Overload Collision Target";
    begin
        Target.Resolve(5);
    end;

    procedure CallControl()
    var
        Ctrl: Codeunit "Overload Collision Control";
    begin
        Ctrl.Solo(5);
    end;
}
