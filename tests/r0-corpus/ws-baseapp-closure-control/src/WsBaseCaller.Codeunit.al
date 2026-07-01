// beyond-1B.3b Task 5.5 fixture (NEGATIVE/CONTROL): identical shape to
// ws-baseapp-closure/ (same Base App .app present in .alpackages, same call),
// but this app.json has NO `application` field at all -- no implicit MS-tier
// dependency is injected, so Base App stays OUT of the closure and
// `BaseRec.DoBaseThing()` must stay an honest `Unknown`. Proves the injection
// is gated on the `application` field actually being present/non-empty, not a
// side effect of the Base App `.app` merely sitting in `.alpackages`.
codeunit 50100 "WS Base Caller"
{
    procedure Run()
    var
        BaseRec: Record "Base App Widget";
    begin
        BaseRec.DoBaseThing();
    end;
}
