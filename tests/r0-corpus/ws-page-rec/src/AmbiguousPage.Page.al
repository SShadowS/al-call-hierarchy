// beyond-1B.3b Task 5 fixture (c) — NEGATIVE: cross-app ambiguity. "Amb Table"
// is declared as a Table in BOTH dependency apps (PageRecLibA and PageRecLibB,
// see .alpackages/) — neither is this workspace's own app, so
// `resolve_object_ref` must DECLINE (`Ambiguous`), never guess one of the two.
// The implicit `Rec` stays `Record{table: None}`, so the non-builtin
// `Rec.Bar()` stays honest `Unknown` — picking either dependency's table would
// be the cardinal sin (a false `Source` edge).
page 50963 "Ambiguous Page"
{
    SourceTable = "Amb Table";

    layout
    {
        area(Content)
        {
        }
    }

    trigger OnOpenPage()
    begin
        Rec.Bar();
    end;
}
