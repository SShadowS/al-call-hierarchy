// beyond-1B.3b Task 3 fixture (b), OWN-OBJECT SHADOW: `SourceTable =
// "IR Table A"` (the SAME table fixture (a) resolves through Step 3) but this
// Page ALSO declares its OWN `procedure GetDisplayText()`. Step 1 (own object) must
// win — the bare `GetDisplayText();` call resolves to THIS PAGE's `GetDisplayText`, never
// reaching Step 3's implicit-Rec fallback, even though the implicit table
// ALSO has a matching `GetDisplayText`.
page 50972 "IR Page B"
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

    procedure GetDisplayText(): Text
    begin
        exit('page-own');
    end;
}
