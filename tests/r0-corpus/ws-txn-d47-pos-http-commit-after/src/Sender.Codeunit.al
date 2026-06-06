codeunit 50000 "D47 Sender"
{
    /// <summary>
    /// Regression case for the COMMIT-carrier dup/misgrade bug: modify, then HTTP,
    /// then Commit. The IO happens inside the open write transaction (the commit is
    /// AFTER the IO), so WRITE_PENDING_AT_EXTERNAL_IO still holds → exactly ONE
    /// CRITICAL finding, graded HTTP (never doubled, never graded as COMMIT).
    /// </summary>
    procedure SendThenCommit()
    var
        Rec: Record "D47 Rec";
        Client: HttpClient;
        Resp: HttpResponseMessage;
    begin
        Rec.Get(10000);
        Rec.Name := 'changed';
        Rec.Modify();
        Client.Get('https://example.test/ping', Resp);
        Commit();
    end;
}
