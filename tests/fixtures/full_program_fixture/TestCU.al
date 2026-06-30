// Fixture for resolve_full_program unit tests.
// Three call obligations in Caller():
//   1. KnownProc()       => Bare, resolves to own-object proc => resolved_source
//   2. UnknownXYZ()      => Bare, no match anywhere           => Unknown
//   3. Codeunit.Run(Dyn) => ObjectRun, runtime variable       => HonestDynamic
// One additional proc (KnownProc) with no calls.
// One publisher routine => Publisher obligation => HonestEmpty EventFlow edge.
codeunit 50200 "TestCU"
{
    var
        Dyn: Integer;

    [IntegrationEvent(false, false)]
    procedure OnMyEvent()
    begin
    end;

    procedure Caller()
    begin
        KnownProc();
        UnknownXYZ();
        Codeunit.Run(Dyn);
    end;

    procedure KnownProc()
    begin
    end;
}
