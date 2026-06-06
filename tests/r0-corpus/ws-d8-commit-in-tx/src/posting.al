codeunit 50108 D8PostingChain
{
    procedure PostSalesDoc()
    var
        Header: Record "Sales Header";
        Line: Record "Sales Line";
    begin
        Header.Get('001');
        Header."Status Posted" := true;
        Header.Modify();
        Line.SetRange("Document No.", Header."No.");
        Line.ModifyAll("Status Posted", true);
        OnAfterPostSalesDoc(Header);
        Header."Last Posting Date" := Today;
        Header.Modify();
    end;

    [IntegrationEvent(false, false)]
    procedure OnAfterPostSalesDoc(Header: Record "Sales Header")
    begin
    end;
}

table 36 "Sales Header"
{
    fields
    {
        field(1; "No."; Code[20]) { }
        field(50; "Status Posted"; Boolean) { }
        field(60; "Last Posting Date"; Date) { }
    }
}

table 37 "Sales Line"
{
    fields
    {
        field(1; "Document No."; Code[20]) { }
        field(50; "Status Posted"; Boolean) { }
    }
}

table 50100 "Audit Entry"
{
    fields
    {
        field(1; "Entry No."; Integer) { }
        field(2; "Doc No."; Code[20]) { }
    }
}
