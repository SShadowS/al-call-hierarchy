codeunit 50000 "D47 Sender"
{
    /// <summary>
    /// No preceding write — the HTTP call runs with no open write transaction.
    /// WRITE_PENDING_AT_EXTERNAL_IO does not hold → ZERO findings.
    /// </summary>
    procedure JustFetch()
    var
        Client: HttpClient;
        Resp: HttpResponseMessage;
    begin
        Client.Get('https://example.test/ping', Resp);
    end;
}
