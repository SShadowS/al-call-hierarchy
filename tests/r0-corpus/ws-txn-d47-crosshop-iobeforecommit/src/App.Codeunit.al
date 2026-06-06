codeunit 50000 "D47 Caller"
{
    /// <summary>
    /// Cross-hop EXTERNAL_IO_BEFORE_COMMIT shape: this routine writes a record,
    /// then calls a callee that performs the HTTP (the IO is reached cross-hop),
    /// then commits. The ordering pass produces an EXTERNAL_IO_BEFORE_COMMIT fact
    /// resolved to the HTTP occurrence across the hop into "D47 Worker".
    ///
    /// In v1 there is no CommitBehavior model, so the commit grades only as
    /// `assumed_effective` (never `proven_effective`) and the guarantee is
    /// `validForRefutation:false` → gradeGuarantee returns "suppressed" → D47
    /// emits NOTHING. This fixture exercises the cross-hop ordering path + the
    /// EXTERNAL_IO_BEFORE_COMMIT label end-to-end and pins the v1 suppression.
    /// </summary>
    procedure WriteThenRemoteCommit()
    var
        Rec: Record "D47 Rec";
        Worker: Codeunit "D47 Worker";
    begin
        Rec.Get(10000);
        Rec.Name := 'changed';
        Rec.Modify();
        Worker.DoHttp();
        Commit();
    end;
}

codeunit 50001 "D47 Worker"
{
    procedure DoHttp()
    var
        Client: HttpClient;
        Resp: HttpResponseMessage;
    begin
        Client.Get('https://example.test/ping', Resp);
    end;
}
