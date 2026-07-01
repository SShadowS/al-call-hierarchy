// beyond-1B.3b Task 3 fixture (g), NEGATIVE — `with`-block: `SourceTable =
// "IR With Target Table"` (which DOES declare a matching `GetNameW`), but the
// bare `GetNameW();` call sits inside `with OtherRec do begin ... end` where
// `OtherRec` is a DIFFERENT record (`"IR With Other Table"`). The with-guard
// (`WithState::InsideWith`) must skip Step 3 entirely — the call must NOT
// resolve to `"IR With Target Table".GetNameW`; it stays honest `Unknown`
// (`GetNameW` is not a builtin either).
page 50984 "IR Page G"
{
    SourceTable = "IR With Target Table";

    layout
    {
        area(Content)
        {
        }
    }

    trigger OnOpenPage()
    var
        OtherRec: Record "IR With Other Table";
    begin
        with OtherRec do begin
            GetNameW();
        end;
    end;
}
