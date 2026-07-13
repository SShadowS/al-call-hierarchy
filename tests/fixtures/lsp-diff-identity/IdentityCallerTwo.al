// Calls ONLY the codeunit's "Shared Name Two".DoSomething() — the page's
// own version (SharedPageTwo.al) is deliberately never called, so
// legacy's collapsed identity slot wrongly credits it with this call.
codeunit 50312 "Identity Caller Two"
{
    procedure CallIt()
    var
        "Shared Name Two": Codeunit "Shared Name Two";
    begin
        "Shared Name Two".DoSomething();
    end;
}
