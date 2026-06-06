codeunit 50101 "XE Worker"
{
    // DoStuff: Commit() then Error() — commit is on the error path only
    // (no normal return path from DoStuff after Commit runs).
    // Per spec J2: commit does NOT precede success-return of DoStuff.
    procedure DoStuff()
    begin
        Commit();
        Error('Always aborts');
    end;
}
