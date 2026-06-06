codeunit 51000 "D34 Demo"
{
    // FLAGGED (high): direct Commit inside a for-loop.
    procedure DirectCommitInLoop()
    var
        i: Integer;
    begin
        for i := 1 to 10 do begin
            DoWork(i);
            Commit();
        end;
    end;

    // FLAGGED (critical): Commit inside a nested loop (depth >= 2).
    procedure NestedCommit()
    var
        i: Integer;
        j: Integer;
    begin
        for i := 1 to 10 do
            for j := 1 to 10 do
                Commit();
    end;

    // FLAGGED (medium, transitive): in-loop call to a callee that commits.
    procedure TransitiveCommit()
    var
        i: Integer;
    begin
        for i := 1 to 10 do
            Persist(i);
    end;

    // NOT FLAGGED: Commit outside the loop is fine.
    procedure CommitAfterLoop()
    var
        i: Integer;
    begin
        for i := 1 to 10 do
            DoWork(i);
        Commit();
    end;

    local procedure DoWork(_n: Integer) begin end;

    local procedure Persist(_n: Integer)
    begin
        Commit();
    end;
}
