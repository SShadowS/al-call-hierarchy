// (a) Record-member shadow: `R.FieldNo('No.')` must resolve to `Acme.FieldNo`
// (Evidence::Source), NOT the `Record::fieldno` catalog builtin.
codeunit 50951 "ShadowCallerA"
{
    procedure CallA()
    var
        R: Record Acme;
    begin
        R.FieldNo('No.');
    end;
}
