codeunit 63101 "D1 Jobs"
{
    procedure ProcessAll()
    var
        i: Integer;
    begin
        for i := 1 to 10 do
            ProcessOne();
    end;

    local procedure ProcessOne()
    var
        Customer: Record Customer;
    begin
        Customer.FindSet();
    end;

    procedure DirectLoop()
    var
        Customer: Record Customer;
        i: Integer;
    begin
        for i := 1 to 10 do
            Customer.Get('C0001');
    end;

    procedure SafeLoop()
    var
        Customer: Record Customer;
        i: Integer;
    begin
        for i := 1 to 10 do
            Customer.SetRange("No.", 'C0001');
    end;

    procedure TwoCallsSameCallee()
    var
        i: Integer;
    begin
        for i := 1 to 10 do begin
            ProcessOne();
            ProcessOne();
        end;
    end;

    procedure NestedLoop()
    var
        Customer: Record Customer;
        i: Integer;
        j: Integer;
    begin
        for i := 1 to 10 do
            for j := 1 to 10 do
                Customer.FindSet();
    end;
}
