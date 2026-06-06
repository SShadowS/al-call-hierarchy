table 50100 "Sales Header"
{
    fields { field(1; "No."; Code[20]) { } }
    keys { key(PK; "No.") { Clustered = true; } }
}

table 50101 "Document Log"
{
    fields { field(1; "Document No."; Code[20]) { } }
    keys { key(PK; "Document No.") { Clustered = true; } }
}
