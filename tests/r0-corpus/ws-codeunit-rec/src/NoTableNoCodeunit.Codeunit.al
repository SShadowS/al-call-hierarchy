// beyond-1B.3b Task 6 fixture (b) — NEGATIVE: no `TableNo` property at all.
// A Codeunit only gets an implicit Rec entity when `TableNo` is declared, so
// this stays the honest `Unknown` (not `Record{table: None}` — there is no
// Record entity to type in the first place).
codeunit 50972 "No Table No Codeunit"
{
    trigger OnRun()
    begin
        Rec.Foo();
    end;
}
