codeunit 65101 "D3 Reports"
{
    procedure MissingLoad()
    var
        Customer: Record Customer;
    begin
        Customer.FindSet();
        if Customer.Name <> '' then
            Customer.Name := Customer.Name;
    end;

    procedure CompleteLoad()
    var
        Customer: Record Customer;
    begin
        Customer.SetLoadFields(Name);
        Customer.FindSet();
        if Customer.Name <> '' then
            Customer.Name := Customer.Name;
    end;

    procedure IncompleteLoad()
    var
        Customer: Record Customer;
    begin
        Customer.SetLoadFields(Name);
        Customer.FindSet();
        if Customer.Name <> '' then
            Customer.Address := Customer.Address;
    end;

    procedure ResetBail()
    var
        Customer: Record Customer;
    begin
        Customer.SetLoadFields(Name);
        Customer.FindSet();
        Customer.Reset();
        if Customer.Name <> '' then
            Customer.Name := Customer.Name;
    end;

    procedure PartialThenCallee()
    var
        Customer: Record Customer;
    begin
        Customer.SetLoadFields("No.");
        Customer.FindSet();
        EnrichCustomer(Customer);
    end;

    local procedure EnrichCustomer(var C: Record Customer)
    begin
        if C.Name <> '' then
            C.Name := C.Name;
    end;

    procedure ResetThenFind()
    var
        Customer: Record Customer;
    begin
        Customer.SetLoadFields(Name);
        Customer.Reset();
        Customer.FindSet();
        if Customer.Name <> '' then
            Customer.Name := Customer.Name;
    end;

    procedure AddAfterReset()
    var
        Customer: Record Customer;
    begin
        Customer.SetLoadFields(Name);
        Customer.Reset();
        Customer.AddLoadFields(Address);
        Customer.FindSet();
        if Customer.Address <> '' then
            Customer.Address := Customer.Address;
    end;
}
