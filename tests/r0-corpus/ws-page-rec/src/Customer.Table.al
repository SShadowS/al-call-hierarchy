// beyond-1B.3b Task 5 fixture (a): the table a `SourceTable`-typed Page's
// implicit `Rec` must resolve to. `GetDisplayName` is a NON-builtin procedure
// — before the Task 5 fix, `CustomerCard`'s `Rec.GetDisplayName()` was an
// honest `Unknown` (Page implicit Rec always carried `Record{table: None}`).
table 50960 "Customer"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }

    procedure GetDisplayName(): Text
    begin
        exit("No.");
    end;
}
