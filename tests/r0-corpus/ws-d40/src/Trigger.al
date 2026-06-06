// Table extension whose OnAfterInsert trigger forwards the implicit `Rec` to
// a sibling procedure that mutates the record without loading. The procedure
// lives in the SAME tableextension object so the call is bare-resolved, and
// the binding carries sourceKind = "implicit-rec".
//
// D40 must NOT flag this: `Rec` inside a table trigger is loaded by the AL
// runtime before the trigger fires — there is no caller in source code that
// could "Get it" first. The implicit-rec narrowing covers exactly this case.
tableextension 50411 "D40 Cust Ext" extends Customer
{
    trigger OnAfterInsert()
    begin
        TouchHelper(Rec);
    end;

    local procedure TouchHelper(var Cust: Record Customer)
    begin
        Cust.Modify();
    end;
}
