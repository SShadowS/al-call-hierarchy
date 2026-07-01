// beyond-1B.3b Task 5 fixture (d) support: a second table, DISTINCT from
// "Customer", whose procedure a local `var Rec: Record "Other Table"` must
// resolve to (shadowing the page's own `SourceTable = Customer`).
table 50964 "Other Table"
{
    fields
    {
        field(1; "Code"; Code[20]) { }
    }

    procedure OtherProc(): Text
    begin
        exit("Code");
    end;
}
