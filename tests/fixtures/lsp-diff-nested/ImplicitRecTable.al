// ImplicitRecResolved probe (CDO layer-2b): a table procedure, called
// BAREWORD from a page bound to this table via SourceTable — no qualifier
// at all, resolved through the page's IMPLICIT `Rec` binding, not a local
// same-object procedure. See ImplicitRecPage.al.
table 50313 "Implicit Rec Table"
{
    fields
    {
        field(1; "No."; Code[20]) { }
    }
    keys
    {
        key(PK; "No.") { }
    }

    procedure SetBackgroundPDF()
    begin
    end;

    procedure RefreshCache()
    begin
    end;
}
