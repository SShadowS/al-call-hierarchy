codeunit 50100 "CSR Caller"
{
    var
        Helper: Codeunit "CSR Caller";

    trigger OnRun()
    begin
        DirectTarget();          // bare call, same object → resolved
        Helper.DirectTarget();   // member call on codeunit variable → resolved (Spec 2 Task 7: global codeunit var resolves)
        RunByVariable();
    end;

    procedure DirectTarget()
    begin
        Commit();                // direct commit — so OnRun inherits commit via both direct-call and variable-typed-call
    end;

    local procedure RunByVariable()
    var
        CodeunitId: Integer;
    begin
        CodeunitId := 50100;
        Codeunit.Run(CodeunitId); // dynamic target → dynamic-target
    end;
}

codeunit 50102 "CSR Member Caller"
{
    var
        Target: Codeunit "CSR Caller";

    procedure CallViaVariable()
    begin
        Target.DirectTarget();   // ONLY member call path to commit — variable-typed-call witness hop
    end;
}
