// Case (a) SELF: an object's OWN `local procedure`, called via a `Record`
// variable of the object's own type from inside the SAME object.
//
// AL semantics: `local` restricts a procedure to the DECLARING OBJECT. A
// call from within that same object — even through an explicit `Rec.`-style
// qualifier — is always legal. Expected: COMPILES. Fresh-engine expected
// route: Evidence::Source, target = LocSelfFoo.DoWork.
table 52600 "LocSelfFoo"
{
    procedure Wrapper()
    var
        R: Record LocSelfFoo;
    begin
        R.DoWork();
    end;

    local procedure DoWork()
    begin
    end;
}
