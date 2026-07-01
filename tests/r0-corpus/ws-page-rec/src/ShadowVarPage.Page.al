// beyond-1B.3b Task 5 fixture (d): a LOCAL `var Rec: Record "Other Table"`
// shadows the implicit `Rec` (step 2 of `infer_receiver_type` runs before
// step 3's implicit-Rec/SourceTable resolution). Even though this page's own
// `SourceTable = Customer`, `Rec.OtherProc()` must resolve against the
// DECLARED type "Other Table" — never against Customer.
page 50965 "Shadow Var Page"
{
    SourceTable = Customer;

    layout
    {
        area(Content)
        {
        }
    }

    trigger OnOpenPage()
    var
        Rec: Record "Other Table";
    begin
        Rec.OtherProc();
    end;
}
