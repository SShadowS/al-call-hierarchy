codeunit 51200 "D36 Demo"
{
    // FLAGGED: SetLoadFields after Get, with no later load on Customer.
    procedure SetLoadFieldsAfterGet()
    var
        Customer: Record Customer;
    begin
        Customer.Get('C0001');
        Customer.SetLoadFields(Name);
    end;

    // NOT FLAGGED: SetLoadFields BEFORE the Get is the correct placement.
    procedure SetLoadFieldsBeforeGet()
    var
        Customer: Record Customer;
    begin
        Customer.SetLoadFields(Name);
        Customer.Get('C0001');
    end;

    // NOT FLAGGED: SetLoadFields between two loads — it prepares the second one.
    procedure SetLoadFieldsBetweenLoads()
    var
        Customer: Record Customer;
    begin
        Customer.Get('C0001');
        Customer.SetLoadFields(Name);
        Customer.FindFirst();
    end;

    // NOT FLAGGED: SetLoadFields with no prior load is D3's domain, not D36's.
    procedure NoPriorLoad()
    var
        Customer: Record Customer;
    begin
        Customer.SetLoadFields(Name);
    end;
}
