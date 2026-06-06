codeunit 66101 "E2E Engine"
{
    trigger OnRun()
    begin
        RunBatch();
        ReportNames();
    end;

    procedure RunBatch()
    var
        i: Integer;
    begin
        for i := 1 to 100 do begin
            LoadCustomer();
            OnAfterRunIteration();
        end;
    end;

    local procedure LoadCustomer()
    var
        Customer: Record Customer;
    begin
        Customer.FindSet();
    end;

    procedure ReportNames()
    var
        Customer: Record Customer;
    begin
        Customer.SetLoadFields("No.");
        Customer.FindSet();
        if Customer.Name <> '' then
            Customer.Address := Customer.Name;
    end;

    [IntegrationEvent(false, false)]
    procedure OnAfterRunIteration()
    begin
    end;
}
