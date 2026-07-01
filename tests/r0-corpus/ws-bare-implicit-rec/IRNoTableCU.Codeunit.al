// beyond-1B.3b Task 3 fixture (h), NEGATIVE — no implicit table: a plain
// Codeunit (no `TableNo`) with a bare `Foo();` call to a name that is neither
// its own procedure nor a global builtin. Step 3's strict-kind guard excludes
// Codeunit entirely (structurally, before any table lookup is attempted) —
// stays honest `Unknown`.
codeunit 50985 "IR No Table CU"
{
    trigger OnRun()
    begin
        Foo();
    end;
}
