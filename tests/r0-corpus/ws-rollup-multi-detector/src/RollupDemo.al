// ws-rollup-multi-detector — anti-degenerate fixture for the terminal-format rollup.
// D1 (db-op-in-loop) + D5 (set-based-opportunity) + D10 (self-modifying-loop) all
// fire on the single Customer.Modify() at the same (file, line, column, table).
// The repeat…Modify…Next pattern with no callSites, no conditional DB ops, and
// the iterating record variable satisfies all three detectors simultaneously.

table 50500 "Rollup Customer"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; Name; Text[100]) { }
        field(3; Status; Option) { OptionMembers = ,Active,Inactive; }
    }
    keys { key(PK; "No.") { } }
}

codeunit 50500 "Rollup Demo"
{
    /// D1 + D5 + D10 all fire on the Modify() call on line 28.
    /// D1 : Modify is a db-write inside the repeat loop.
    /// D5 : exactly one Modify on the iterating record (Cust), no callSites in
    ///      the loop, all other in-loop ops are Next (ALLOWED) — qualifies for ModifyAll.
    /// D10: Modify runs on the record variable (Cust) that Next() is advancing.
    procedure BulkMarkActive()
    var
        Cust: Record "Rollup Customer";
    begin
        if Cust.FindSet() then
            repeat
                Cust.Status := 1;
                Cust.Modify();
            until Cust.Next() = 0;
    end;
}
