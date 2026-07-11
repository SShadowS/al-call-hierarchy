// T0.3/T1.3 fixture: the statically-named RunModal target for the
// Report-keyword population exercised in AuditCaller.Codeunit.al.
// `OnPreReport` is declared so T1.3's entry-trigger dispatch fix has a real
// Source target to prove against (a genuine `Run` edge into user code, not
// merely an Opaque boundary route).
report 50952 "Audit Target Report"
{
    dataset
    {
        dataitem(D1; "Audit Target Table")
        {
        }
    }

    trigger OnPreReport()
    begin
    end;
}
