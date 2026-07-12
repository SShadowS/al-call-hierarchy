table 50102 "LSP Incr Table"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; Description; Text[100])
        {
            trigger OnValidate()
            var
                Beta: Codeunit "Beta";
            begin
                Beta.Process();
            end;
        }
    }
    keys
    {
        key(PK; "No.") { }
    }
}
