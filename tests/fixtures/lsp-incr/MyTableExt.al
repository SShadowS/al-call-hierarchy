tableextension 50103 "LSP Incr Table Ext" extends "LSP Incr Table"
{
    fields
    {
        field(50; "Extra Field"; Text[50])
        {
            trigger OnValidate()
            var
                Alpha: Codeunit "Alpha";
            begin
                Alpha.Calc(1);
            end;
        }
    }
}
