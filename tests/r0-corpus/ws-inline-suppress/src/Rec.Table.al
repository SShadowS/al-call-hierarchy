table 50100 "IS Rec"
{
    fields
    {
        field(1; "No."; Integer) { }
        field(2; "Name"; Text[50]) { }
    }
    keys
    {
        key(PK; "No.") { Clustered = true; }
    }
}
