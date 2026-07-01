// beyond-1B.3b Task 3 fixture (f), NEGATIVE — page-intrinsic collision:
// declares a procedure named `Update`, arity 0 — same name+arity as the
// bare-callable `PageInstance` intrinsic (`member_catalog`'s `PAGE_INSTANCE`
// set). A bare `Update()` call from a Page sourced at THIS table must NOT
// assume the table wins — fail-closed to `Unknown`.
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
