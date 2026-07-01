// beyond-1B.3b Task 3 fixture (k), NEGATIVE — strict-kind (Report): dataitem
// sources "IR Strict Kind Table" (which DOES declare `Foo`), but Report is
// structurally EXCLUDED from Step 3's kind guard (Report implicit-Rec is
// per-dataitem-scoped, a separate future task — see the module doc). The bare
// `Foo();` call must stay honest `Unknown`.
report 50991 "IR Strict Kind Report"
{
    dataset
    {
        dataitem(D1; "IR Strict Kind Table")
        {
            trigger OnAfterGetRecord()
            begin
                Foo();
            end;
        }
    }
}
