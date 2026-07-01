// beyond-1B.3b Task 3 fixture (g) tables: `IR With Target Table` is the Page's
// OWN `SourceTable` (what Step 3 WOULD resolve `GetNameW()` to, absent the
// `with`-guard); `IR With Other Table` is an UNRELATED table, used only as
// the type of the `with`-receiver variable in `IRPageG.Page.al`.
table 50982 "IR With Target Table"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }

    procedure GetNameW(): Text
    begin
        exit("No.");
    end;
}

table 50983 "IR With Other Table"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }
}
