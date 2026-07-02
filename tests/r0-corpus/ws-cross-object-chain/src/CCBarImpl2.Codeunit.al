// plan v2.1 Task 3 fixture — implementer 2 of 2 for `ICCBar` (see
// `CCBarImpl.Codeunit.al` for the first) — makes `ICCBar` genuinely
// polymorphic (2 implementers), the (N1) NEGATIVE control.
codeunit 51205 "CC Bar Impl B" implements ICCBar
{
    procedure GetHelper(): Codeunit "CC Helper"
    var
        H: Codeunit "CC Helper";
    begin
        exit(H);
    end;
}
