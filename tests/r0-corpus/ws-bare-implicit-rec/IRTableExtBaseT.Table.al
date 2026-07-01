// Review-fix fixture (Task 3 NEEDS-FIXES finding): base table for a
// TableExtension-caller Step-3 positive proof. Deliberately declares NO
// procedure matching the bare call under test (`SharedProc`) — Step 2
// (extension base, which searches ONLY this table's own procs) must decline
// so the call can only resolve through Step 3's `resolve_in_table_scope`
// (base table UNION its visible TableExtensions), proving the union half of
// the search, not the base-table half (already proven by fixture (a)).
table 50993 "IR TableExt Base T"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }
}
