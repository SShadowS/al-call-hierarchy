// beyond-1B.3b Task 3 fixture (c), POSITIVE: bare `ExtProc();` (declared only
// on the visible TableExtension `IRTableAExtC.TableExtension.al`, not on the
// base table or this page) must resolve through Step 3 to the extension's
// `ExtProc`, `Evidence::Source`.
page 50974 "IR Page C"
{
    SourceTable = "IR Table A";

    layout
    {
        area(Content)
        {
        }
    }

    trigger OnOpenPage()
    begin
        ExtProc();
    end;
}
