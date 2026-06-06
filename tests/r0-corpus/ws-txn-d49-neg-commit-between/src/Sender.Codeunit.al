codeunit 50000 "D49 Sender"
{
    /// <summary>
    /// NEGATIVE: Commit() between the Modify() and Message() — no pending write at UI.
    /// Must produce ZERO d49 findings.
    /// </summary>
    procedure ModifyCommitMessage()
    var
        Rec: Record "D49 Rec";
    begin
        Rec.Get(10000);
        Rec.Name := 'changed';
        Rec.Modify();
        Commit();
        Message('Done');
    end;
}
