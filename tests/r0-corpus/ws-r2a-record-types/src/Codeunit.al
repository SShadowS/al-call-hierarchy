// Record-variable + operation resolution cases for Phase R2a.

codeunit 50900 "R2a Records"
{
    var
        // OBJECT-GLOBAL record var — exercises the unified-scope fallback (the op
        // below uses it without a local declaration).
        GlobalCust: Record Customer;

    procedure DeclaredVars()
    var
        Cust: Record Customer;        // → resolves to 50900
        Sales: Record "Sales Line";   // quoted base table → resolves to 50901
        TempCust: Record Customer temporary; // temporary modifier → still resolves to 50900
        Missing: Record "No Such Table";     // unresolved → tableId OMITTED
    begin
        Cust.FindSet();      // op tableId from Cust → 50900
        Sales.FindFirst();   // op tableId from Sales → 50901
        TempCust.Insert();   // op tableId from TempCust → 50900
        Missing.Get();       // op tableId UNRESOLVED → OMITTED
    end;

    procedure GlobalScopeOp()
    begin
        // No local var named GlobalCust; the op resolves via features.variables
        // (the unified lexical scope includes object globals).
        GlobalCust.Modify();
    end;
}

page 50900 "R2a Cust Card"
{
    PageType = Card;
    SourceTable = Customer;

    trigger OnOpenPage()
    begin
        // implicit Rec on a Page → resolves to the SourceTable (Customer / 50900).
        Rec.Modify();
    end;
}
