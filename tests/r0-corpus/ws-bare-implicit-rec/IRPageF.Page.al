// beyond-1B.3b Task 3 fixture (f), NEGATIVE: bare `Update();` collides
// between the implicit table's own `Update` procedure
// (`IRPageIntrinsicTable.Table.al`) and the bare-callable `PageInstance`
// intrinsic `Update` — must stay honest `Unknown`.
page 50981 "IR Page F"
{
    SourceTable = "IR Page Intrinsic Table";

    layout
    {
        area(Content)
        {
        }
    }

    trigger OnOpenPage()
    begin
        Update();
    end;
}
