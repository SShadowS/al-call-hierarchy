codeunit 50000 "D47 Sender"
{
    /// <summary>
    /// Commit between the write and the HTTP — the transaction is durable before the
    /// IO, so the write is no longer pending at the external IO point → ZERO findings.
    /// </summary>
    procedure SendAfterCommit()
    var
        Rec: Record "D47 Rec";
        Client: HttpClient;
        Resp: HttpResponseMessage;
    begin
        Rec.Get(10000);
        Rec.Name := 'changed';
        Rec.Modify();
        Commit();
        Client.Get('https://example.test/ping', Resp);
    end;
}
