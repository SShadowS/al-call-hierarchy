// `C: Codeunit "80"` is a QUOTED name reference to the codeunit literally
// named "80" (id 50100), NOT the numeric id 80 (`RealById`). `C.P()` must
// resolve, via `Evidence::Source`, to Named80.Codeunit.al's `P()` — never to
// `RealById` (id 80), which has no `P` at all.
codeunit 50101 Caller
{
    procedure Trigger()
    var
        C: Codeunit "80";
    begin
        C.P();
    end;
}
