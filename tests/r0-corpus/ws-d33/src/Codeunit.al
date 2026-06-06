codeunit 50900 "D33 Demo"
{
    // FLAGGED (critical): DeleteAll with no prior filter on Customer.
    procedure NukeAllCustomers()
    var
        Customer: Record Customer;
    begin
        Customer.DeleteAll();
    end;

    // FLAGGED (high): ModifyAll with no prior filter.
    procedure ModifyAllCustomers()
    var
        Customer: Record Customer;
    begin
        Customer.ModifyAll(Name, 'X');
    end;

    // NOT FLAGGED: SetRange precedes DeleteAll, narrowing the set.
    procedure NarrowedDelete()
    var
        Customer: Record Customer;
    begin
        Customer.SetRange("No.", 'C0001');
        Customer.DeleteAll();
    end;

    // FLAGGED: Reset clears the previously-set filter, so DeleteAll runs unfiltered.
    procedure FilterThenResetThenDelete()
    var
        Customer: Record Customer;
    begin
        Customer.SetRange("No.", 'C0001');
        Customer.Reset();
        Customer.DeleteAll();
    end;

    // NOT FLAGGED: by-var parameter — caller is responsible for filters.
    procedure WithVarParam(var Customer: Record Customer)
    begin
        Customer.DeleteAll();
    end;

    // NOT FLAGGED: declared with the `temporary` keyword — RecordVariable.tempState is
    // {kind: "known", value: true}, propagated onto the DeleteAll op by
    // routine-indexer.ts. No name-based heuristics — tree-sitter sees the keyword
    // directly.
    procedure TrulyTemporary()
    var
        TempCustomer: Record Customer temporary;
    begin
        TempCustomer.DeleteAll();
    end;
}
