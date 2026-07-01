// beyond-1B.3b Task 6 fixture (e): a LOCAL `var Rec: Record "Other Table"`
// shadows the implicit Rec (step 2 of `infer_receiver_type` runs before
// step 3's implicit-Rec/TableNo resolution). Even though this codeunit's own
// `TableNo = Item`, `Rec.OtherProc()` must resolve against the DECLARED type
// "Other Table" — never against Item.
codeunit 50976 "Shadow Var Codeunit"
{
    TableNo = Item;

    trigger OnRun()
    var
        Rec: Record "Other Table";
    begin
        Rec.OtherProc();
    end;
}
