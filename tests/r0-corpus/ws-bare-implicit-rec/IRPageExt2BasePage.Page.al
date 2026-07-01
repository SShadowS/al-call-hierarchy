// Review-fix fixture (Task 3 NEEDS-FIXES finding): base page for the
// PageExtension-caller Step-3 positive proof. `SourceTable = "IR PageExt2 Src
// Table"`, but this page DELIBERATELY declares NO `OnlyOnTable` of its own —
// Step 2 (extension base, which searches ONLY this page's own procs) must
// decline so the call in `IRPageExt2Ext.PageExtension.al` can only resolve
// through Step 3's `resolve_pageext_base_source_table` (base page's inherited
// SourceTable).
page 50997 "IR PageExt2 Base Page"
{
    SourceTable = "IR PageExt2 Src Table";

    layout
    {
        area(Content)
        {
        }
    }
}
