// beyond-1B.3b Task 3 fixture (e), NEGATIVE: bare `StrLen(Txt)` (arity 1)
// collides between the implicit table's own `StrLen` procedure
// (`IRBuiltinCollideTable.Table.al`) and the global `strlen` intrinsic — must
// stay honest `Unknown` (the builtin/intrinsic PROBE-THEN-DECIDE guard).
page 50979 "IR Page E"
{
    SourceTable = "IR Builtin Collide Table";

    layout
    {
        area(Content)
        {
        }
    }

    trigger OnOpenPage()
    var
        Txt: Text;
    begin
        StrLen(Txt);
    end;
}
