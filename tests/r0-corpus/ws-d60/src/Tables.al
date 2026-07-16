table 50934 "D60 Item"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; Name; Text[100]) { }
    }
    keys { key(PK; "No.") { } }
}

table 50936 "D60 Ref"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; Val; Integer) { }
    }
    keys { key(PK; "No.") { } }
}
