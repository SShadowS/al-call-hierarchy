table 50307 "Nested Trigger Table"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; "Background PDF"; Blob)
        {
            trigger OnValidate()
            begin
                SetBackgroundPDF();
            end;
        }
    }
    keys
    {
        key(PK; "No.") { }
    }

    procedure SetBackgroundPDF()
    begin
    end;
}
