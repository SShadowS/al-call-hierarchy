// Dataitem-receivers plan, Task 1 — ReportExtension coverage: the `modify()`
// lowerer gap (d) and the extends-target base-dataitem fallback (e).
reportextension 51701 "RD Report Ext" extends "RD Base Report"
{
    dataset
    {
        // (d): a DATASET `modify(Cust)` trigger on the BASE report's "Cust"
        // dataitem — `RawKind::ModifyModification` carries `Target`, not
        // `Name`, so pre-fix this trigger's `enclosing_member` AND
        // `dataitem_source_table` were both `None`. Post-fix: the lowerer's
        // additive `Target` read populates `enclosing_member = "Cust"` +
        // `in_dataset_modify_context = true`, and the resolver's confirmed-
        // dataset-context fallback (`resolve_dataitem_source_table`, keyed
        // by `enclosing_member`) resolves the implicit Rec via the OWN+BASE
        // merged dataitem map (here: the base's own "Cust" -> "RD Customer").
        modify(Cust)
        {
            trigger OnAfterGetRecord()
            begin
                Rec.GetDisplayName();
            end;
        }
    }

    // (e): the extension has NO dataitems of its own — a bare dataitem-NAME
    // receiver naming the BASE report's "Cust" dataitem must still resolve,
    // via the extends-target base-dataitem fallback (mirrors the
    // PageExtension `SourceTable` inheritance pattern).
    procedure ExtTestBaseDataitemName()
    begin
        Cust.GetDisplayName();
    end;
}
