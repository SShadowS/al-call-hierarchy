// beyond-1B.3b Task 3 fixture (j) support: the base page's `SourceTable`,
// declaring its OWN `Foo` — the implicit-Rec (Step 3) target Step 3 would
// resolve to IF Step 2 (extension base) did not already win first.
table 50987 "IR PageExt Src Table"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }

    procedure Foo(): Text
    begin
        exit('table-foo');
    end;
}
