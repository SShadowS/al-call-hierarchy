// plan v2.1 Task 3 fixture — implementer 1 of 2 for `ICCBar` (see
// `CCBarImpl2.Codeunit.al` for the second).
codeunit 51204 "CC Bar Impl A" implements ICCBar
{
    procedure GetHelper(): Codeunit "CC Helper"
    var
        H: Codeunit "CC Helper";
    begin
        exit(H);
    end;
}
