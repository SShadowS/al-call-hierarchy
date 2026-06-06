codeunit 50000 "D49 Sender"
{
    /// <summary>
    /// POSITIVE: physical Modify() then Page.RunModal() with no commit between.
    /// WRITE_PENDING_AT_UI — refutation grade — must fire at HIGH.
    /// Page.RunModal is a ui-window-open sink.
    /// </summary>
    procedure ModifyThenRunModal()
    var
        Rec: Record "D49 Rec";
    begin
        Rec.Get(10000);
        Rec.Name := 'changed';
        Rec.Modify();
        Page.RunModal(Page::"D49 Sender");
    end;
}
