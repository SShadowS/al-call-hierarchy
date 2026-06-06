codeunit 50200 "D1 Setup Singleton Demo"
{
    // Get on a Setup table inside a loop — BC caches per session, so D1 should
    // downgrade this finding to `info`.
    procedure InLoopSetupGet()
    var
        SalesSetup: Record "Sales Receivables Setup";
        i: Integer;
    begin
        for i := 1 to 100 do
            SalesSetup.Get();
    end;

    // Get on a non-Setup table inside a loop — must keep `medium` severity.
    procedure InLoopCustomerGet()
    var
        Customer: Record Customer;
        i: Integer;
    begin
        for i := 1 to 100 do
            Customer.Get('C0001');
    end;

    // FindSet on a Setup table — the heuristic only applies to Get, so this stays medium.
    procedure InLoopSetupFindSet()
    var
        SalesSetup: Record "Sales Receivables Setup";
        i: Integer;
    begin
        for i := 1 to 10 do
            SalesSetup.FindSet();
    end;
}
