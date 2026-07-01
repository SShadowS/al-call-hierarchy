// beyond-1B.3b Task 6 fixture (c) — NEGATIVE: `Subtype = TestRunner` never
// declares `TableNo` (test/test-runner codeunits have no statically-typed
// implicit Rec at all — unhandled even in the legacy L3 engine, zero
// TestRunner support anywhere). `ObjectNode` does not track `Subtype`
// specially; the `TableNo`-presence check alone already produces the correct
// honest `Unknown` here, with nothing fabricated.
codeunit 50973 "Test Runner Codeunit"
{
    Subtype = TestRunner;

    trigger OnRun()
    begin
        Rec.Bar();
    end;
}
