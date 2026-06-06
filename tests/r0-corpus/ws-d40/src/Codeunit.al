codeunit 50410 "D40 Demo"
{
    // FLAGGED (high): helper mutates without load; caller doesn't load first.
    procedure DoesNotLoadThenForwardsToMutator()
    var
        Customer: Record Customer;
    begin
        MutatingHelper(Customer);
    end;

    // FLAGGED (medium): helper reads without load; caller doesn't load first.
    procedure DoesNotLoadThenForwardsToReader()
    var
        Customer: Record Customer;
    begin
        ReadingHelper(Customer);
    end;

    // NOT FLAGGED: caller loaded before forwarding.
    procedure LoadsBeforeForwarding()
    var
        Customer: Record Customer;
    begin
        Customer.Get('C0001');
        MutatingHelper(Customer);
    end;

    // NOT FLAGGED: helper loads its own param.
    procedure ForwardsToSelfLoader()
    var
        Customer: Record Customer;
    begin
        SelfLoadingHelper(Customer);
    end;

    // NOT FLAGGED: temporary record (no DB load concept).
    procedure TempForward()
    var
        TempCustomer: Record Customer temporary;
    begin
        MutatingHelper(TempCustomer);
    end;

    local procedure MutatingHelper(var Cust: Record Customer)
    begin
        Cust.Modify();
    end;

    local procedure ReadingHelper(var Cust: Record Customer)
    begin
        Message(Cust.Name);
    end;

    local procedure SelfLoadingHelper(var Cust: Record Customer)
    begin
        Cust.Get('C0001');
        Cust.Modify();
    end;
}
