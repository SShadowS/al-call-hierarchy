codeunit 50901 "Processor"
{
    trigger OnRun()
    begin
        Process();
    end;

    procedure Process()
    var
        Customer: Record Customer;
        SalesLine: Record "Sales Line";
    begin
        Customer.Get('C0001');
        Helper();
    end;

    local procedure Helper()
    begin
    end;
}
