// beyond-1B.3b Task 5 fixture (a) — POSITIVE: `SourceTable = Customer` types
// the implicit `Rec`. `Rec.GetDisplayName()` (a non-builtin table procedure)
// must resolve to `Customer.GetDisplayName` (Evidence::Source). `Rec.FieldCaption`
// (a genuine Record-catalog builtin, table-independent per the receiver.rs
// module doc) must stay classified as a builtin regardless of the table
// resolving. `Rec.SetRange` is included too — a `record_op_names` call, which
// dispatches through the SEPARATE implicit-trigger fan-out (not `resolve_member`'s
// catalog), so it must not be mis-reclassified `Source`/`Unknown` by the fix.
page 50961 "Customer Card"
{
    SourceTable = Customer;

    layout
    {
        area(Content)
        {
        }
    }

    trigger OnOpenPage()
    begin
        Rec.GetDisplayName();
        Rec.FieldCaption(1);
        Rec.SetRange("No.", '10000');
    end;
}
