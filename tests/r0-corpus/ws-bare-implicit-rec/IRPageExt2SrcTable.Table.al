// Review-fix fixture (Task 3 NEEDS-FIXES finding): the SourceTable of
// `IRPageExt2BasePage.Page.al`, declaring `OnlyOnTable` — a procedure that
// exists ONLY here, never on the base page. This is the Step 3 target for
// the PageExtension-caller positive proof in `IRPageExt2Ext.PageExtension.al`.
table 50996 "IR PageExt2 Src Table"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }

    procedure OnlyOnTable(): Text
    begin
        exit('only-on-table');
    end;
}
