codeunit 50101 "LO LoopThenCommit"
{
    procedure PostLoopThenCommit()
    var
        Rec: Record "LO Rec";
        Done: Boolean;
    begin
        Done := false;
        repeat
            Rec.Init();
            Rec.Insert(true);
        until Done;
        Commit();
    end;
}
