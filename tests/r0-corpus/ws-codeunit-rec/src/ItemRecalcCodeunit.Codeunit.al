// beyond-1B.3b Task 6 fixture (a) — POSITIVE: `TableNo = Item` types the
// implicit `Rec`. `Rec.Recalculate()` (a non-builtin table procedure) must
// resolve to `Item.Recalculate` (Evidence::Source). `Rec.FieldCaption` (a
// genuine Record-catalog builtin, table-independent per the receiver.rs
// module doc) must stay classified as a builtin regardless of the table
// resolving. `Rec.SetRange` is included too — a `record_op_names` call, which
// dispatches through the SEPARATE implicit-trigger fan-out (not
// `resolve_member`'s catalog), so it must not be mis-reclassified
// `Source`/`Unknown` by the fix.
codeunit 50971 "Item Recalc"
{
    TableNo = Item;

    trigger OnRun()
    begin
        Rec.Recalculate();
        Rec.FieldCaption(1);
        Rec.SetRange("No.", '10000');
    end;
}
