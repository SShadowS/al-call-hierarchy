codeunit 50102 "XE Caller"
{
    // Run: calls DoStuff (which always errors) then makes an HTTP call.
    // The HTTP call is unreachable on any normal path that went through DoStuff.
    // → NO commit ≺ http cross-hop edge (success-return restriction J2).
    procedure Run()
    var
        Worker: Codeunit "XE Worker";
        Client: HttpClient;
        Response: HttpResponseMessage;
    begin
        Worker.DoStuff();
        Client.Get('https://example.com/notify', Response);
    end;
}
