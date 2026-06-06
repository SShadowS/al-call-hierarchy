codeunit 62101 "Cycle CU"
{
    procedure Ping()
    var
        Customer: Record Customer;
    begin
        Customer.FindSet();
        Pong();
    end;

    procedure Pong()
    begin
        Ping();
    end;
}
