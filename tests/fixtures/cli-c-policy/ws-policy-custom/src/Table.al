// A local table whose name matches the "* Order" glob pattern in the policy.
table 50200 "Custom Order"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; "Description"; Text[100]) { }
    }
    keys { key(PK; "No.") { Clustered = true; } }
}
