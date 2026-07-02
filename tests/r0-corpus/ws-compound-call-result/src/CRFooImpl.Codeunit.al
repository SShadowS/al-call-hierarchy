// beyond-1B.3b Task 3 fixture — the SOLE implementer of `ICRFoo`, so
// `GetIFoo().Bar()` fans out to exactly one route.
codeunit 51002 "CR Foo Impl" implements ICRFoo
{
    procedure Bar()
    begin
    end;
}
