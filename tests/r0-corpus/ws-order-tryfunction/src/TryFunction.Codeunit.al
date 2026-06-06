codeunit 50101 "TF TryFunction"
{
    [TryFunction]
    procedure TryInsertAndCommit()
    var
        Rec: Record "TF Rec";
    begin
        Rec.Init();
        Rec.Insert(true);
        Commit();
    end;
}
