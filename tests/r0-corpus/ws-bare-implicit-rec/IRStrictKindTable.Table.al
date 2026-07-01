// beyond-1B.3b Task 3 fixture (k) support: a table declaring `Foo`, referenced
// (directly or via `TableNo`) by the strict-kind NEGATIVE fixtures below —
// proves the strict ObjectKind guard excludes Report/Codeunit EVEN WHEN a
// real, resolvable, same-name+arity table procedure exists.
table 50990 "IR Strict Kind Table"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }

    procedure Foo(): Text
    begin
        exit('strict-kind-table-foo');
    end;
}
