codeunit 50000 "D47 Clean Publisher"
{
    /// <summary>
    /// Negative: publisher raises a non-isolated event WITHOUT dirtying the
    /// transaction first (no Modify/Insert/Delete). No write-pending pattern →
    /// EXTERNAL_IO_IN_EVENT_SUBSCRIBER_TXN advisory must NOT fire.
    /// D47 must emit ZERO findings.
    /// </summary>
    procedure NotifyOnly()
    begin
        OnAfterNotify();
    end;

    [IntegrationEvent(false, false)]
    procedure OnAfterNotify()
    begin
    end;
}
