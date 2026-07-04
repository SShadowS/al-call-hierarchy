// beyond-1B.3b Task 3 fixture (f) — page-intrinsic collision: declares a
// procedure named `Update`, arity 0 — same name+arity as `PageInstance`'s
// catalog entry (`member_catalog`'s `PAGE_INSTANCE` set). Since
// pageext-merge-and-final-residual plan Task 2 grounded `Update` as having
// NO bare-call form anywhere in AL (always `CurrPage.Update()`/receiver-
// qualified), a bare `Update()` call from a Page sourced at THIS table now
// correctly resolves to THIS procedure — see `IRPageF.Page.al`.
table 50980 "IR Page Intrinsic Table"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }

    procedure Update(): Text
    begin
        exit('table-update');
    end;
}
