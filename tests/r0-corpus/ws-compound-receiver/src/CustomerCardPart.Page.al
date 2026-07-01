// beyond-1B.3b Task 7 fixture — the SUBPAGE target of "Customer Card"'s
// `Lines` part control. `RefreshLines` is the non-builtin subpage-instance
// procedure that `CurrPage.Lines.Page.RefreshLines()` (fixture a, in
// CustomerCard.Page.al) must resolve to.
page 50990 "Customer Card Part"
{
    PageType = ListPart;

    layout
    {
        area(Content)
        {
        }
    }

    procedure RefreshLines()
    begin
    end;
}
