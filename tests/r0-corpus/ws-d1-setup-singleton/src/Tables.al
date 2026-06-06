table 50200 "Sales Receivables Setup"
{
    fields
    {
        field(1; "Primary Key"; Code[10]) { }
        field(10; "Default Tax Group"; Code[20]) { }
    }
    keys { key(PK; "Primary Key") { } }
}

table 50201 Customer
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; Name; Text[100]) { }
    }
    keys { key(PK; "No.") { } }
}
