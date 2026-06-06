codeunit 50420 "D41 Demo"
{
    // FLAGGED: filter → forwarded to reset helper → FindFirst operates on unfiltered set
    procedure FilterThenCallResetThenFind()
    var
        Customer: Record Customer;
    begin
        Customer.SetRange("No.", 'C0001');
        ResettingHelper(Customer);
        Customer.FindFirst();
    end;

    // NOT FLAGGED: re-filtered between Reset helper and FindFirst
    procedure FilterThenCallResetThenReFilterThenFind()
    var
        Customer: Record Customer;
    begin
        Customer.SetRange("No.", 'C0001');
        ResettingHelper(Customer);
        Customer.SetRange("No.", 'C0001');
        Customer.FindFirst();
    end;

    // NOT FLAGGED: no post-call filter-sensitive op (intentional reset helper)
    procedure FilterThenCallNoPostUse()
    var
        Customer: Record Customer;
    begin
        Customer.SetRange("No.", 'C0001');
        ResettingHelper(Customer);
    end;

    // NOT FLAGGED: no prior filter
    procedure CallResetThenFindNoPriorFilter()
    var
        Customer: Record Customer;
    begin
        ResettingHelper(Customer);
        Customer.FindFirst();
    end;

    local procedure ResettingHelper(var Cust: Record Customer)
    begin
        Cust.Reset();
    end;
}
