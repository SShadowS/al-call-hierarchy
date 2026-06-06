codeunit 61101 "Compose Caller"
{
    procedure RunAll()
    begin
        DoDbWork();
    end;

    local procedure DoDbWork()
    var
        Customer: Record Customer;
    begin
        Customer.FindSet();
    end;
}
