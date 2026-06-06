codeunit 50101 "TU TransitiveCommit"
{
    procedure CommitWorker()
    var
        Rec: Record "TU Rec";
    begin
        Rec.Init();
        Rec.Insert(true);
        Commit();
    end;

    procedure CallerWithTransitiveCommit()
    begin
        CommitWorker();
    end;
}
