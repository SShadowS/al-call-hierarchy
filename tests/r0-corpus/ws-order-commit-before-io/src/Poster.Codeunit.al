codeunit 50101 "OC Poster"
{
    procedure PostAndCommit()
    var
        Rec: Record "OC Rec";
    begin
        Rec.Init();
        Rec.Insert(true);
        Commit();
    end;
}
