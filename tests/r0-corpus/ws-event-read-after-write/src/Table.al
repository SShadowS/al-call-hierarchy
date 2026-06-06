table 50100 Inventory
{
    fields { field(1; "No."; Code[20]) { } field(2; Qty; Decimal) { } }
    keys { key(PK; "No.") { Clustered = true; } }
}
