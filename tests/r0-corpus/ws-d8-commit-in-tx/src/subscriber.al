codeunit 50109 D8BadSubscriber
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::D8PostingChain, 'OnAfterPostSalesDoc', '', true, true)]
    local procedure HandlePosted(Header: Record "Sales Header")
    var
        Audit: Record "Audit Entry";
    begin
        Audit.Init();
        Audit."Doc No." := Header."No.";
        Audit.Insert();
        Commit();
    end;
}
