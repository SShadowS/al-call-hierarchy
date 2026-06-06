table 72100 "Order Line"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; Quantity; Decimal) {
            trigger OnValidate()
            var
                AnotherLine: Record "Order Line";
            begin
                // DB op inside the implicit OnValidate trigger
                AnotherLine.FindSet();
            end;
        }
    }
    keys { key(PK; "No.") { } }
}
