// beyond-1B.3b Task 5 fixture (e) — originally NEGATIVE (Report/
// ReportExtension implicit-Rec was unconditionally `Record{table: None}`,
// per-dataitem scoping being a future task). UPDATED by the dataitem-
// receivers plan (Task 1): a report's implicit Rec is now ROUTINE-
// CONTEXTUAL — `RoutineDecl.dataitem_source_table` threads the enclosing
// dataitem's source table into `infer_implicit_rec`'s Report/ReportExtension
// arm, mirroring Page/PageExtension's `SourceTable` precedent. This report's
// dataitem sources "Customer" (which DOES declare `GetDisplayName`), so
// `Rec.GetDisplayName()` inside the dataitem trigger now correctly resolves
// `Evidence::Source` — see `ws_page_rec_report_dataitem_resolves_via_
// dataitem_source_table` in `tests/program_resolve_harness.rs`.
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
