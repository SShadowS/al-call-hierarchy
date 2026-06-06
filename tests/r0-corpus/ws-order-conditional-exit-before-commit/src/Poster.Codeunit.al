codeunit 50101 "CEX Poster"
{
    // THE BUG CASE: conditional exit before Commit.
    // Commit does NOT postdominate all success returns:
    //   - if Skip=true  → exit runs, returns normally WITHOUT Commit
    //   - if Skip=false → Commit runs, returns normally
    // Sound result: COMMIT_DOMINATES_RETURN must NOT be emitted.
    procedure ConditionalExitThenCommit(Skip: Boolean)
    var
        Rec: Record "CEX Rec";
    begin
        if Skip then
            exit;
        Rec.Insert(true);
        Commit();
    end;

    // STRAIGHT-LINE case: no conditional exit before Commit.
    // Commit postdominates all success returns.
    // Sound result: COMMIT_DOMINATES_RETURN must be emitted.
    procedure StraightLineCommit()
    var
        Rec: Record "CEX Rec";
    begin
        Rec.Insert(true);
        Commit();
    end;

    // ERROR-GUARD case: Error() before Commit (not a normal return).
    // The Error guard does NOT pollute dominatesSuccessReturn.
    // Sound result: COMMIT_DOMINATES_RETURN must be emitted (Error is not a normal return).
    procedure ErrorGuardThenCommit(Bad: Boolean)
    var
        Rec: Record "CEX Rec";
    begin
        if Bad then
            Error('Aborted');
        Rec.Insert(true);
        Commit();
    end;
}
