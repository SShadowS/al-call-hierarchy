table 50100 Item
{
    fields { field(1; "No."; Code[20]) { } }
    keys { key(PK; "No.") { Clustered = true; } }

    trigger OnInsert()
    begin
        Commit();
    end;
}
