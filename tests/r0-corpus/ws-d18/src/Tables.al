table 50400 Customer
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; Name; Text[100]) { }
        field(3; Status; Option) { OptionMembers = Open,Blocked,Closed; }
    }
    keys { key(PK; "No.") { } }
}
