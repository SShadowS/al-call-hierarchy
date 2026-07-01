// beyond-1B.3b Task 5.5 fixture (POSITIVE): the workspace declares `application`
// in app.json (24.0.0.0) but Base App is NOT listed in `dependencies[]` -- real
// BC apps never do. `BaseRec.DoBaseThing()` is a non-builtin table procedure
// that only exists in the synthetic Base App `.app` (.alpackages). Before the
// Task 5.5 fix, Base App was systematically absent from the closure, so
// `resolve_object_ref` returned `OutOfClosure` and this call was an honest
// `Unknown`. After the fix, the implicit `application` -> MS_APPLICATION_TIER
// dependency wires Base App into the closure and the call resolves to the
// Base App table procedure. Evidence is `Opaque` here because this synthetic
// `.app` is symbol-only (ABI boundary, no embedded source); a real ShowMyCode
// Base App with embedded source would resolve `Evidence::Source`.
codeunit 50100 "WS Base Caller"
{
    procedure Run()
    var
        BaseRec: Record "Base App Widget";
    begin
        BaseRec.DoBaseThing();
    end;
}
