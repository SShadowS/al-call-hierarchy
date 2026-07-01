// beyond-1B.3b Task 6 fixture (a): the table a `TableNo`-typed Codeunit's
// implicit `Rec` must resolve to. `Recalculate` is a NON-builtin procedure —
// before the Task 6 fix, a Codeunit's implicit Rec always carried `Unknown`
// (Codeunit had no arm in `infer_implicit_rec` at all).
table 50970 "Item"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }

    procedure Recalculate(): Boolean
    begin
        exit(true);
    end;
}
