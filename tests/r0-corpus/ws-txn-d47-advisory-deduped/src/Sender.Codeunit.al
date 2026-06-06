codeunit 50000 "D47 Deduped Sender"
{
    /// <summary>
    /// Dedupe case: Modify (dirty write) → HTTP POST → Commit(). Both
    /// WRITE_PENDING_AT_EXTERNAL_IO (critical) AND EXTERNAL_IO_BEFORE_COMMIT advisory
    /// (§0.5, info) would fire on the same HTTP POST occurrence. The dedupe rule in D47
    /// suppresses the advisory when WRITE_PENDING already fires on the same IO
    /// occurrence → exactly ONE finding (critical), not two.
    /// </summary>
    procedure ModifyPostThenCommit()
    var
        Rec: Record "D47 Rec";
        Client: HttpClient;
        Content: HttpContent;
        Resp: HttpResponseMessage;
    begin
        Rec.Get(10000);
        Rec.Name := 'changed';
        Rec.Modify();
        Client.Post('https://example.test/events', Content, Resp);
        Commit();
    end;
}
