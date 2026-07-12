// LegacyIdentityCollapse probe, shape 2: TWO DIFFERENT FIELDS on the SAME
// table, each with their OWN "OnValidate" trigger. Legacy's `definitions`
// map has no enclosing-member discriminator at all — both triggers share
// the identical (object="Two Triggers Table", routine="OnValidate") key
// and collide (last-write-wins), even though they're two entirely
// different, unrelated pieces of code.
table 50306 "Two Triggers Table"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(2; "Field A"; Text[50])
        {
            trigger OnValidate()
            begin
            end;
        }
        field(3; "Field B"; Text[50])
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
}
