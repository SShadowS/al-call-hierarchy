// beyond-1B.3b Task 3 fixture (d), NEGATIVE: bare `Dup();` collides between
// the two sibling TableExtensions declared in `IRTableAExtD.TableExtension.al`
// — must stay honest `Unknown` (never pick one arbitrarily).
page 50977 "IR Page D"
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
        Dup();
    end;
}
