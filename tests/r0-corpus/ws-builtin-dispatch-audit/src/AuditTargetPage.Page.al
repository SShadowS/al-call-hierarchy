// T0.3/T1.3 fixture: the statically-named RunModal target for the
// keyword-receiver and declared-page-variable populations exercised in
// AuditCaller.Codeunit.al. `OnOpenPage` is declared so T1.3's entry-trigger
// dispatch fix has a real Source target to prove against (a genuine `Run`
// edge into user code, not merely an Opaque boundary route).
page 50951 "Audit Target Page"
{
    layout
    {
        area(Content)
        {
        }
    }

    trigger OnOpenPage()
    begin
    end;
}
