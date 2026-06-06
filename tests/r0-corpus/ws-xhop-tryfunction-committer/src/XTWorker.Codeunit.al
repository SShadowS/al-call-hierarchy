codeunit 50101 "XT Worker"
{
    // TryCommit: a [TryFunction] — commits inside a try boundary.
    // Per spec J6: [TryFunction] internals are barriers.
    // The commit effectiveness is "proven_errors" from the caller's perspective
    // (the error is caught by the TryFunction wrapper; the commit never escapes
    // the try boundary in the standard sense — it is "trapped").
    [TryFunction]
    procedure TryCommit()
    var
        Rec: Record "XT Rec";
    begin
        Rec.Init();
        Rec.Insert(true);
        Commit();
    end;
}
