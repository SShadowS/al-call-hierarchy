codeunit 50102 "XA Caller"
{
    // Run: calls P() (always errors), then Commit().
    // The Commit is unreachable on any execution that entered Run normally,
    // because P always errors before returning.
    // → No composition through the always-error callee (J5 returnability).
    procedure Run()
    var
        Worker: Codeunit "XA Worker";
    begin
        Worker.P();
        Commit();
    end;
}
