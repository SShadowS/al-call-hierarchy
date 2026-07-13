// ImplicitTriggerEdge OUTGOING-axis probe (CDO re-run finding, final
// residual): extends "Implicit Trigger Table"'s existing "Amount" field
// with a SECOND OnValidate trigger — see ImplicitTrigger.al's doc. Both
// this trigger and the base table's own "Amount".OnValidate legitimately
// fire together on every `Rec.Validate(Amount, ...)` call (real AL
// semantics: every extension's field trigger fires alongside the base
// one), giving the resolver's ImplicitTrigger fan-out a genuine multicast
// candidate set of 2 — never a duplicate-emission artifact.
tableextension 50315 "Implicit Trigger Table Ext" extends "Implicit Trigger Table"
{
    fields
    {
        modify("Amount")
        {
            trigger OnValidate()
            begin
            end;
        }
    }
}
