// plan v2.1 Task 3 fixture — leaf target table for the SOURCE cross-object
// chain positive: `Helper.GetCustomer(No).Name()` must type the PREFIX
// receiver as `Record{table: Some(CCCustomer)}` and resolve `Name` (a
// non-builtin table procedure) `Source`, exact target id.
table 51200 "CC Customer"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }
    keys
    {
        key(PK; "No.") { Clustered = true; }
    }

    procedure Name(): Text
    begin
        exit('x');
    end;
}
