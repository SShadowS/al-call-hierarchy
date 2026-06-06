codeunit 50990 "Chain Test"
{
    procedure Top()
    var
        Customer: Record Customer;
    begin
        Hop1(Customer);
    end;

    local procedure Hop1(var Cust: Record Customer)
    begin
        Hop2(Cust);
    end;

    local procedure Hop2(var Cust: Record Customer)
    begin
        Hop3(Cust);
    end;

    local procedure Hop3(var Cust: Record Customer)
    begin
        Cust.Modify(); // mutates without load — root of the requiresLoadedAtEntry=yes chain
    end;
}
