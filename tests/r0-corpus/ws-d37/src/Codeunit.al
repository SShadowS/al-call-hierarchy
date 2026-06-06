codeunit 51400 "D37 Demo"
{
    // FLAGGED: Validate then routine exits with no Modify.
    procedure ValidateWithoutModify()
    var
        Customer: Record Customer;
    begin
        Customer.Get('C0001');
        Customer.Validate(Blocked, true);
    end;

    // NOT FLAGGED: Validate followed by Modify persists the change.
    procedure ValidateThenModify()
    var
        Customer: Record Customer;
    begin
        Customer.Get('C0001');
        Customer.Validate(Blocked, true);
        Customer.Modify();
    end;

    // FLAGGED: Validate then Get reloads the record, discarding the validate.
    procedure ValidateThenReload()
    var
        Customer: Record Customer;
    begin
        Customer.Get('C0001');
        Customer.Validate(Blocked, true);
        Customer.Get('C0002');
    end;

    // NOT FLAGGED: Validate then Insert persists for the new record.
    procedure ValidateThenInsert()
    var
        Customer: Record Customer;
    begin
        Customer.Init();
        Customer."No." := 'NEW';
        Customer.Validate(Name, 'X');
        Customer.Insert();
    end;

    // NOT FLAGGED: Customer is forwarded to a helper after Validate — could persist there.
    procedure ForwardedAfterValidate()
    var
        Customer: Record Customer;
    begin
        Customer.Get('C0001');
        Customer.Validate(Blocked, true);
        Helper(Customer);
    end;

    local procedure Helper(var Customer: Record Customer)
    begin
        Customer.Modify();
    end;

    // FLAGGED: helper is provably non-persisting (just reads); Validate without
    // persist is real. Phase 3 closes the helper-suppression gap for this case.
    procedure ValidateThenNonPersistingHelper()
    var
        Cust: Record Customer;
    begin
        Cust.Get('C0001');
        Cust.Validate(Name, 'X');
        NonPersistingHelper(Cust);
    end;

    local procedure NonPersistingHelper(var Cust: Record Customer)
    begin
        Message(Cust.Name);
    end;

    // NOT FLAGGED (suppressed by helperPersistsUnknown): the forwarded helper is a
    // member call on a Codeunit variable — Phase 1 does not type-track non-record
    // variables, so the dispatch is "member" / unresolved, leaving the binding at
    // "unresolved-callee". D37 must conservatively suppress.
    procedure ValidateThenUnresolvedMemberCall()
    var
        Cust: Record Customer;
        Helper: Codeunit "D37 Helper";
    begin
        Cust.Get('C0001');
        Cust.Validate(Name, 'X');
        Helper.DoSomething(Cust);
    end;
}

codeunit 51401 "D37 Helper"
{
    procedure DoSomething(var Cust: Record Customer)
    begin
        Cust.Modify();
    end;
}
