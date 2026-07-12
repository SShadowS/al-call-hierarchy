table 50311 "Implicit Trigger Table"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }
    keys
    {
        key(PK; "No.") { }
    }

    trigger OnInsert()
    begin
        ImportXML();
    end;

    procedure ImportXML()
    begin
    end;
}
