table 50000 PostingEntry
{
    fields
    {
        field(1; "Entry No."; Integer) { }
        field(2; "No."; Code[20]) { }
        field(3; "Description"; Text[50]) { }
    }

    keys
    {
        key(PK; "Entry No.") { Clustered = true; }
    }
}
