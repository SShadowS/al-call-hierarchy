// Dataitem-receivers plan, Task 1 — the table a report dataitem's implicit
// Rec must resolve to. `GetDisplayName` is a NON-builtin procedure (mirrors
// `ws-page-rec/src/Customer.Table.al`'s identical Page/SourceTable precedent
// — the same real CDO shape, `Customer.GetDisplayName`, now proven for a
// Report's per-dataitem implicit Rec).
table 51710 "RD Customer"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }

    procedure GetDisplayName(): Text
    begin
        exit("No.");
    end;
}
