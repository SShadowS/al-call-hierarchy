// Fixture for the L3-validated semantic golden assertion (1B.3a Task 4).
//
// Four edges in Caller():
//   1. ProcA()             => Bare, own-object     => resolved_source
//   2. NoSuchProc()        => Bare, no match       => Unknown (empty targets)
//   3. Codeunit.Run(CuId)  => ObjectRun, variable  => HonestDynamic (empty targets)
//
// OtherGoldenCU also defines ProcA — tests same-name-different-object:
//   calling ProcA() from SemanticGoldenCU.Caller must resolve to
//   SemanticGoldenCU.ProcA, not OtherGoldenCU.ProcA.
codeunit 50400 "SemanticGoldenCU"
{
    var
        CuId: Integer;

    procedure ProcA()
    begin
    end;

    procedure Caller()
    begin
        ProcA();
        NoSuchProc();
        Codeunit.Run(CuId);
    end;
}

codeunit 50401 "OtherGoldenCU"
{
    // Same name as SemanticGoldenCU.ProcA — same-name-different-object probe.
    procedure ProcA()
    begin
    end;
}
