// Review-fix fixture (Task 3 NEEDS-FIXES finding), POSITIVE — PageExtension
// CALLER reaching Step 3 via the base page's SourceTable: this PageExtension
// (of "IR PageExt2 Base Page") declares its OWN procedure `CallOnlyOnTable`,
// whose body makes a BARE call to `OnlyOnTable()` — declared ONLY on the base
// page's `SourceTable` (`IRPageExt2SrcTable.Table.al`), never on the base
// page itself or on this extension. Step 1 (own object) misses it (not
// declared here); Step 2 (extension base) misses it (the base page has no
// own `OnlyOnTable` — only its SourceTable does); only Step 3
// (`resolve_pageext_base_source_table` → `resolve_in_table_scope`) finds it.
// This is the coverage gap the review finding identified: the ORIGINAL
// Task 3 fixture (j) proved the PRECEDENCE case (base page's OWN proc beats
// the SourceTable's same-named proc, so Step 2 wins and Step 3 is never
// entered) but never proved the case where Step 3 actually FIRES for a
// PageExtension caller.
pageextension 50998 "IR PageExt2 Ext" extends "IR PageExt2 Base Page"
{
    procedure CallOnlyOnTable(): Text
    begin
        exit(OnlyOnTable());
    end;
}
