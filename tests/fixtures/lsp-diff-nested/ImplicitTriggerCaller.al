codeunit 50312 "Implicit Trigger Caller"
{
    procedure DoInsert()
    var
        Rec: Record "Implicit Trigger Table";
    begin
        Rec.Insert(true);
    end;

    // ImplicitTriggerEdge OUTGOING-axis probe (CDO re-run finding, final
    // residual) — see ImplicitTrigger.al/ImplicitTriggerTableExt.al's docs.
    procedure DoValidate()
    var
        Rec: Record "Implicit Trigger Table";
    begin
        Rec.Validate(Amount, 1.5);
    end;
}
