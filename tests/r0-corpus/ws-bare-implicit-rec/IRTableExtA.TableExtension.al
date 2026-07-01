// Review-fix fixture (Task 3 NEEDS-FIXES finding), POSITIVE — TableExtension
// CALLER reaching Step 3 via the sibling-extension union: this TableExtension
// (of "IR TableExt Base T") declares its OWN procedure `CallShared`, whose
// body makes a BARE call to `SharedProc()` — declared ONLY on the SIBLING
// TableExtension `IR TableExt B` (`IRTableExtB.TableExtension.al`), never on
// the base table or on this extension itself. Step 1 (own object) misses it
// (not declared here); Step 2 (extension base) misses it (searches only the
// base table's own procs, which has none); only Step 3
// (`resolve_in_table_scope`'s base-table-UNION-extensions search) finds it,
// via the sibling `IR TableExt B`. This is the coverage gap the review
// finding identified: the ORIGINAL Task 3 fixture set (c) proved the
// base-table half of the union (a Page's bare call resolving to a visible
// TableExtension) but never proved a TableExtension ITSELF as the CALLER
// reaching Step 3 through the union with a SIBLING extension.
tableextension 50995 "IR TableExt A" extends "IR TableExt Base T"
{
    procedure CallShared(): Text
    begin
        exit(SharedProc());
    end;
}
