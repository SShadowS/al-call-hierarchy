codeunit 50000 "D47 Sender"
{
    /// <summary>
    /// Flagship case: modifies a record (dirty write transaction) then makes an
    /// HTTP call with NO commit between. WRITE_PENDING_AT_EXTERNAL_IO — refutation
    /// grade — must fire at CRITICAL.
    /// </summary>
    procedure SendAfterModify()
    var
        Rec: Record "D47 Rec";
        Client: HttpClient;
        Resp: HttpResponseMessage;
    begin
        Rec.Get(10000);
        Rec.Name := 'changed';
        Rec.Modify();
        Client.Get('https://example.test/ping', Resp);
    end;
}
