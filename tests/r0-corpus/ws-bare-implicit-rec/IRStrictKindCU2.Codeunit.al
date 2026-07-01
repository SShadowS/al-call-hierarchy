// beyond-1B.3b Task 3 fixture (k), NEGATIVE — strict-kind (Codeunit +
// TableNo): declares `TableNo = "IR Strict Kind Table"` (which DOES have a
// matching `Foo`) — the STRONGEST proof of the strict-kind guard: even though
// this Codeunit's implicit Rec IS statically typed (`infer_implicit_rec`'s
// Codeunit/TableNo arm, Task 6, already resolves it for EXPLICIT `Rec.Foo()`
// calls), `resolve_bare`'s bare-implicit-dispatch fallback (Step 3) never
// applies to Codeunit at all — bare `Foo();` must stay honest `Unknown`, not
// silently pick up the TableNo table's procedure.
codeunit 50992 "IR Strict Kind CU2"
{
    TableNo = "IR Strict Kind Table";

    trigger OnRun()
    begin
        Foo();
    end;
}
