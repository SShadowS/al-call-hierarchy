codeunit 50116 D11Patterns
{
    // Set-based: ModifyAll after SetRange — no per-record state needed. Must NOT flag.
    procedure SetBasedBulk()
    var Customer: Record Customer;
    begin
        Customer.SetRange("No.", 'X');
        Customer.ModifyAll("Last Date Modified", Today);
    end;

    // Init→Validate→Insert: the canonical AL new-record pattern. Must NOT flag.
    procedure InitInsertPattern()
    var Customer: Record Customer;
    begin
        Customer.Init();
        Customer."No." := 'Y';
        Customer.Validate("Last Date Modified", Today);
        Customer.Insert(true);
    end;

    // Modify with no prior Get and no Init: still a real bug. MUST flag.
    procedure RealBug()
    var Customer: Record Customer;
    begin
        Customer."No." := 'Z';
        Customer.Modify();
    end;
}

table 18 Customer
{
    fields {
        field(1; "No."; Code[20]) { }
        field(50; "Last Date Modified"; Date) { }
    }
}
