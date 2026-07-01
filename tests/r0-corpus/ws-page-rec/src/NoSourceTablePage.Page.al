// beyond-1B.3b Task 5 fixture (b) — NEGATIVE: no `SourceTable` property at
// all. The implicit `Rec` must stay `Record{table: None}`, so a non-builtin
// call (`Rec.Foo`, not declared anywhere) stays honest `Unknown`.
page 50962 "No Source Table Page"
{
    layout
    {
        area(Content)
        {
        }
    }

    trigger OnOpenPage()
    begin
        Rec.Foo();
    end;
}
