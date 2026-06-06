codeunit 50000 "D47 Isolated Publisher"
{
    /// <summary>
    /// Negative: Modify() + ISOLATED integration event. Isolated events run in a
    /// separate transaction, so the subscriber's IO cannot be inside the publisher's
    /// write transaction. D47 must emit ZERO findings.
    /// </summary>
    procedure ProcessAndNotify()
    var
        Rec: Record "D47 Isolated Rec";
    begin
        Rec.Get(10000);
        Rec.Name := 'updated';
        Rec.Modify();
        OnAfterProcessIsolated();
    end;

    [IntegrationEvent(false, false, true)]
    procedure OnAfterProcessIsolated()
    begin
    end;
}
