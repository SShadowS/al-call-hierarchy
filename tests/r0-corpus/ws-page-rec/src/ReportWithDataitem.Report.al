// beyond-1B.3b Task 5 fixture (e) — NEGATIVE: Report/ReportExtension are
// EXCLUDED from this task (a report's implicit Rec is scoped PER-DATAITEM,
// not a single object-level SourceTable the way Page/PageExtension are — see
// the Report/ReportExtension arm comment in `infer_implicit_rec`). Even
// though this report's dataitem sources "Customer" (which DOES declare
// `GetDisplayName`), `Rec.GetDisplayName()` inside the dataitem trigger must
// stay honest `Unknown`, not resolve.
report 50966 "Report With Dataitem"
{
    dataset
    {
        dataitem(Cust; Customer)
        {
            trigger OnAfterGetRecord()
            begin
                Rec.GetDisplayName();
            end;
        }
    }
}
