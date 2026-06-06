codeunit 50102 "XT Caller"
{
    // Run: calls TryCommit.
    // The TryFunction is a barrier — no refutation-grade label must be emitted.
    procedure Run()
    var
        Worker: Codeunit "XT Worker";
    begin
        Worker.TryCommit();
    end;
}
