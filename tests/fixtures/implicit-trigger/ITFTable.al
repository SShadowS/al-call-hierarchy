// 1B.3b Task 1: synthetic ImplicitTrigger target-set fixture.
//
// Table 50500 "ITFTable" declares OnInsert/OnModify/OnDelete triggers.
// TableExtension 50501 "ITFTableExt" ALSO declares OnInsert (fan-out target —
// every insert into ITFTable must fire BOTH the base table's and the
// extension's OnInsert). Codeunit 50502 "ITFCaller" performs one Insert,
// Modify, and Delete on a local `Record ITFTable` variable.
//
// Expected fresh ImplicitTrigger resolution (frozen in
// `tests/goldens/semantic-edges/implicit-trigger-fixture.json` — L3-independent,
// no oracle involved):
//   • MyRec.Insert() -> {Table ITFTable.OnInsert, TableExtension ITFTableExt.OnInsert}
//   • MyRec.Modify() -> {Table ITFTable.OnModify}
//   • MyRec.Delete() -> {Table ITFTable.OnDelete}
table 50500 "ITFTable"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }

    trigger OnInsert()
    begin
    end;

    trigger OnModify()
    begin
    end;

    trigger OnDelete()
    begin
    end;
}

tableextension 50501 "ITFTableExt" extends "ITFTable"
{
    trigger OnInsert()
    begin
    end;
}

codeunit 50502 "ITFCaller"
{
    procedure DoStuff()
    var
        MyRec: Record "ITFTable";
    begin
        MyRec.Insert();
        MyRec.Modify();
        MyRec.Delete();
    end;
}
