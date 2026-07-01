// Review-fix fixture (Task 3 NEEDS-FIXES finding): SIBLING TableExtension of
// "IR TableExt Base T", declaring the procedure the bare call in
// `IRTableExtA.TableExtension.al` resolves to. Neither the base table nor the
// CALLING extension (`IR TableExt A`) declares `SharedProc` — only this
// extension does, so a bare call to it can ONLY be found via Step 3's
// visibility-scoped union search (base table ∪ ALL its TableExtensions), not
// Step 1 (own object) or Step 2 (extension base, base-table-only).
tableextension 50994 "IR TableExt B" extends "IR TableExt Base T"
{
    procedure SharedProc(): Text
    begin
        exit('shared-b');
    end;
}
