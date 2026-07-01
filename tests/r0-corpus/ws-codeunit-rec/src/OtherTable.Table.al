// beyond-1B.3b Task 6 fixture (e) support: a second table, DISTINCT from
// "Item", whose procedure a local `var Rec: Record "Other Table"` must
// resolve to (shadowing the codeunit's own `TableNo = Item`).
table 50975 "Other Table"
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
