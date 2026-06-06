codeunit 50000 "D47 Sender"
{
    /// <summary>
    /// Physical record write, then a FILE write before the Commit. The file IO
    /// happens inside the open write transaction → WRITE_PENDING_AT_EXTERNAL_IO on a
    /// FILE effect. FILE direction is currently "unknown" (taxonomy gap) so
    /// gradeGuarantee yields HIGH.
    /// </summary>
    procedure ExportAfterModify()
    var
        Rec: Record "D47 Rec";
        F: File;
    begin
        Rec.Get(10000);
        Rec.Name := 'changed';
        Rec.Modify();
        F.WriteAllText('line', TextEncoding::UTF8);
        Commit();
    end;
}
