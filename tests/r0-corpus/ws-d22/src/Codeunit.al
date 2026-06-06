codeunit 50800 "D22 Demo"
{
    // FLAGGED: reads FlowField without prior CalcFields.
    procedure ReadFlowFieldWithoutCalc()
    var
        Customer: Record Customer;
        Amount: Decimal;
    begin
        Customer.Get('C0001');
        Amount := Customer."Balance (LCY)";
    end;

    // NOT FLAGGED: CalcFields is called before the FlowField read.
    procedure ReadFlowFieldAfterCalc()
    var
        Customer: Record Customer;
        Amount: Decimal;
    begin
        Customer.Get('C0001');
        Customer.CalcFields("Balance (LCY)");
        Amount := Customer."Balance (LCY)";
    end;

    // NOT FLAGGED: reads a Normal (non-Flow) field.
    procedure ReadNormalField()
    var
        Customer: Record Customer;
        N: Text;
    begin
        Customer.Get('C0001');
        N := Customer.Name;
    end;
}
