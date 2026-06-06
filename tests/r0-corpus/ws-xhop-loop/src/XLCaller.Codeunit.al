codeunit 50102 "XL Caller"
{
    procedure Run()
    var
        Rec: Record "XL Rec";
        Worker: Codeunit "XL Worker";
        Done: Boolean;
    begin
        Done := false;
        repeat
            Rec.Init();
            Rec.Insert(true);
            Worker.DoCommit();
        until Done;
    end;
}
