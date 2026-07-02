// plan v2.1 Task 3 fixture — the SOLE implementer of `ICCFoo`.
codeunit 51202 "CC Foo Impl" implements ICCFoo
{
    procedure GetHelper(): Codeunit "CC Helper"
    var
        H: Codeunit "CC Helper";
    begin
        exit(H);
    end;
}
