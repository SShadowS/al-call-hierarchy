codeunit 50101 "TC TwoCommits"
{
    procedure PostTwice()
    var
        Rec: Record "TC Rec";
    begin
        Rec.Init();
        Rec.Insert(true);
        Commit();
        Rec.Modify(true);
        Commit();
    end;
}
