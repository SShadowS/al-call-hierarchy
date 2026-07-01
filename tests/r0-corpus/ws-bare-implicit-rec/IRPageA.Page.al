// beyond-1B.3b Task 3 fixture (a), POSITIVE: `SourceTable = "IR Table A"`; the
// page declares NO own `GetDisplayText` — the bare (unqualified) call `GetDisplayText();`
// in `OnOpenPage` must fall through Step 1 (own object — absent) and Step 2
// (not an extension) to Step 3 (implicit-Rec) and resolve to
// `"IR Table A".GetDisplayText`, `Evidence::Source`. Before Task 3 this was an
// honest `Unknown` (`resolve_bare`'s Step 3 was a TODO).
//
// MUST NOT be resolvable via Step 1 — deliberately no own `GetDisplayText` here (see
// fixture (b), `IRPageB.Page.al`, for the shadow case where it IS declared).
page 50971 "IR Page A"
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
        GetDisplayText();
    end;
}
