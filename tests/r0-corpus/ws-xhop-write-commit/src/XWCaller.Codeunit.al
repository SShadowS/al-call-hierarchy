codeunit 50102 "XW Caller"
{
    procedure Run()
    var
        Rec: Record "XW Rec";
        Worker: Codeunit "XW Worker";
    begin
        Rec.Init();
        Rec.Insert(true);
        Worker.DoCommit();
    end;
}
