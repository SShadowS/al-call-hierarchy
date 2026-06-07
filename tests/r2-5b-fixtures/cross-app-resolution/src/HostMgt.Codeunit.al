codeunit 70000 "Host Mgt"
{
    var
        cust: Record "Dep Customer";

    procedure DriveDeps()
    var
        cu: Codeunit "Dep Mgt";
        localCust: Record "Dep Customer";
    begin
        cu.Compute(1);
        cu.InternalReset();
        cu.LocalHelper();
        cu.Apply(localCust);
        cu.Missing();
        cust.SetRange("No.", '10000');
        cust.FindFirst();
    end;

    [IntegrationEvent(false, false)]
    procedure OnHostStarted()
    begin
    end;
}