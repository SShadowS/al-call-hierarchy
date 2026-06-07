codeunit 70000 "Host Mgt"
{
    var
        cust: Record "Dep Customer";

    procedure DriveDeps()
    var
        cu: Codeunit "Dep Mgt";
    begin
        cu.Compute(1);
        cu.InternalReset();
        cu.LocalHelper();
        cu.Missing();
        cust.SetRange("No.", '10000');
        cust.FindFirst();
    end;

    [IntegrationEvent(false, false)]
    procedure OnHostStarted()
    begin
    end;
}