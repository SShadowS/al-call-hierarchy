codeunit 50000 "D49 Sender"
{
    /// <summary>
    /// POSITIVE: physical Modify() then Message() with no commit between.
    /// WRITE_PENDING_AT_UI — refutation grade — must fire at HIGH.
    /// BC runtime error: "you cannot open a window after modifying the database."
    /// </summary>
    procedure ModifyThenMessage()
    var
        Rec: Record "D49 Rec";
    begin
        Rec.Get(10000);
        Rec.Name := 'changed';
        Rec.Modify();
        Message('Done');
    end;
}
