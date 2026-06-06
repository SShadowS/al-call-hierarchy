codeunit 50102 "XC Caller"
{
    procedure Run(Cond: Boolean)
    var
        Rec: Record "XC Rec";
        Worker: Codeunit "XC Worker";
    begin
        Rec.Init();
        Rec.Insert(true);
        if Cond then
            Worker.DoCommit();
    end;
}
