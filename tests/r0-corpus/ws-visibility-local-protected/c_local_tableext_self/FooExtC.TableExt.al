// Case (c) TableExtension `local` SELF-call: the extension declares its own
// `local procedure` and calls it via `Rec.DoWork()` where `Rec` is typed to
// the BASE table (the only receiver type a TableExtension member can be
// reached through). The CALLING object (FooExtC) is the SAME object that
// declares `DoWork` — this is self, not the cross-object case (b).
//
// AL semantics: legal — a TableExtension's own `local` member is fully
// visible to code within that same extension. Expected: COMPILES.
// Fresh-engine expected route: Evidence::Source, target = FooExtC.DoWork.
tableextension 52621 "FooExtC" extends Foo
{
    procedure Wrapper()
    var
        R: Record Foo;
    begin
        R.DoWork();
    end;

    local procedure DoWork()
    begin
    end;
}
