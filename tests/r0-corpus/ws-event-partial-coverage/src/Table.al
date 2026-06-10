table 50100 Customer
{
    fields { field(1; "No."; Code[20]) { } }
    keys { key(PK; "No.") { Clustered = true; } }
}

table 50101 "Dispatch Setup"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; "Codeunit Id"; Integer) { }
    }
    keys { key(PK; "No.") { Clustered = true; } }
}
