table 50100 Sales
{
    fields { field(1; "No."; Code[20]) { } }
    keys { key(PK; "No.") { Clustered = true; } }
}

table 50101 AuditLog
{
    fields { field(1; "Entry No."; Integer) { } }
    keys { key(PK; "Entry No.") { Clustered = true; } }
}
