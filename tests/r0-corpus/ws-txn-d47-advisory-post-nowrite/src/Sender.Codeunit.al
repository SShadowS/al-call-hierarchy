codeunit 50000 "D47 Advisory Sender"
{
    /// <summary>
    /// Advisory case: HTTP POST is ordered before a Commit() with NO pending write
    /// at the IO (no Modify/Insert before the POST). WRITE_PENDING_AT_EXTERNAL_IO
    /// does NOT fire (no dirty transaction). EXTERNAL_IO_BEFORE_COMMIT advisory
    /// (§0.5) DOES fire at info severity for write-direction POST.
    /// </summary>
    procedure PostThenCommit()
    var
        Client: HttpClient;
        Content: HttpContent;
        Resp: HttpResponseMessage;
    begin
        Client.Post('https://example.test/events', Content, Resp);
        Commit();
    end;
}
