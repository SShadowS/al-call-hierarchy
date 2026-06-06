codeunit 50000 "D47 Evt Publisher"
{
    /// <summary>
    /// Positive: Modify() dirties the transaction, then the non-isolated integration
    /// event fires synchronously. A subscriber may execute inside the publisher's
    /// uncommitted write transaction and issue external IO.
    /// Expect: EXTERNAL_IO_IN_EVENT_SUBSCRIBER_TXN advisory → D47 info finding.
    /// </summary>
    procedure ProcessAndNotify()
    var
        Rec: Record "D47 Evt Rec";
    begin
        Rec.Get(10000);
        Rec.Name := 'updated';
        Rec.Modify();
        OnAfterProcess();
    end;

    [IntegrationEvent(false, false)]
    procedure OnAfterProcess()
    begin
    end;
}
