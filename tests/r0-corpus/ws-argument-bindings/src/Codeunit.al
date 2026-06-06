codeunit 50190 "Argument Bindings Test"
{
    procedure CallVariations()
    var
        Customer: Record Customer;
        Amount: Decimal;
    begin
        HelperLocalRecord(Customer);
        HelperLiteral(42);
        HelperExpression(Amount + 1);
        HelperImplicitRec(Rec);
    end;

    procedure WithVarParam(var Cust: Record Customer)
    begin
        HelperLocalRecord(Cust);
    end;

    procedure WithByValueParam(Cust: Record Customer)
    begin
        HelperLocalRecord(Cust);
    end;

    local procedure HelperLocalRecord(var X: Record Customer) begin end;
    local procedure HelperLiteral(N: Integer) begin end;
    local procedure HelperExpression(N: Decimal) begin end;
    local procedure HelperImplicitRec(var X: Record Customer) begin end;
}
