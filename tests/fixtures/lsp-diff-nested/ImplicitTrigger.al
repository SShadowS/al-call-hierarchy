table 50311 "Implicit Trigger Table"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        // ImplicitTriggerEdge OUTGOING-axis probe (CDO re-run finding, final
        // residual): a field-level OnValidate, ALSO extended by
        // ImplicitTriggerTableExt.al's `modify("Amount")` — same field,
        // second declaring object — so `Rec.Validate(Amount, ...)` fans out
        // to a MULTICAST candidate set of 2 (base + extension), mirroring
        // CDO's real `Table 6175283 "CDO E-Mail Template Header"`.`UpdateTemplateLines`
        // finding (7 distinct field-scoped OnValidate routines sharing one
        // field name across base + extensions).
        field(2; "Amount"; Decimal)
        {
            trigger OnValidate()
            begin
            end;
        }
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
