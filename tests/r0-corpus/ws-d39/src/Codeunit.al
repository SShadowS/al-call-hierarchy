codeunit 50390 "D39 Demo"
{
    // FLAGGED: helper exits dirty, caller doesn't persist after.
    procedure CallerDoesNotPersist()
    var
        Customer: Record Customer;
    begin
        Customer.Get('C0001');
        ValidatesAndExits(Customer);
    end;

    // NOT FLAGGED: caller persists after.
    procedure CallerPersistsAfter()
    var
        Customer: Record Customer;
    begin
        Customer.Get('C0001');
        ValidatesAndExits(Customer);
        Customer.Modify();
    end;

    // NOT FLAGGED: helper persists itself (dirtyAtExit=no).
    procedure HelperPersists()
    var
        Customer: Record Customer;
    begin
        Customer.Get('C0001');
        ValidatesAndPersists(Customer);
    end;

    local procedure ValidatesAndExits(var Cust: Record Customer)
    begin
        Cust.Validate(Name, 'X');
        exit;
    end;

    local procedure ValidatesAndPersists(var Cust: Record Customer)
    begin
        Cust.Validate(Name, 'X');
        Cust.Modify();
    end;
}
