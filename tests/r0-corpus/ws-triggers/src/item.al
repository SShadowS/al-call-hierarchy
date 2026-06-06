table 51100 Item
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; Description; Text[100]) {
            trigger OnValidate()
            begin
            end;
        }
    }
    keys { key(PK; "No.") { } }

    trigger OnModify()
    begin
    end;

    trigger OnInsert()
    begin
    end;
}
