// Case (f) SELF: the declaring table calls its OWN `protected procedure`.
//
// AL semantics: legal — any access level (including `protected`) is always
// visible from within the declaring object itself. Expected: COMPILES.
// Fresh-engine expected route: Evidence::Source, target = Bar.P.
table 52640 "Bar"
{
    procedure Wrapper()
    var
        R: Record Bar;
    begin
        R.P();
    end;

    protected procedure P()
    begin
    end;
}
