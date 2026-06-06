codeunit 50400 "D18 Demo"
{
    // FLAGGED: SetRange with literal value inside a for-loop — hoist outside.
    procedure ConstantSetRangeInLoop()
    var
        Customer: Record Customer;
        i: Integer;
    begin
        for i := 1 to 10 do begin
            Customer.SetRange(Status, Customer.Status::Open);
            Customer.FindFirst();
        end;
    end;

    // FLAGGED: SetFilter with literal value inside a foreach-loop.
    procedure ConstantSetFilterInLoop()
    var
        Customer: Record Customer;
        i: Integer;
    begin
        for i := 1 to 10 do begin
            Customer.SetFilter("No.", '%1', 'C0001');
            Customer.FindFirst();
        end;
    end;

    // NOT FLAGGED: filter value is a loop-dependent expression — non-literal.
    procedure VariableFilterInLoop()
    var
        Customer: Record Customer;
        i: Integer;
    begin
        for i := 1 to 10 do begin
            Customer.SetRange("No.", Format(i));
            Customer.FindFirst();
        end;
    end;

    // NOT FLAGGED: SetRange OUTSIDE the loop is fine.
    procedure SetRangeBeforeLoop()
    var
        Customer: Record Customer;
        i: Integer;
    begin
        Customer.SetRange(Status, Customer.Status::Open);
        for i := 1 to 10 do
            Customer.FindFirst();
    end;
}
