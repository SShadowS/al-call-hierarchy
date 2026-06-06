codeunit 50001 PostingHandler
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::PostingMgr, 'OnBeforePost', '', false, false)]
    local procedure OnBeforePostHandler(var Rec: Record PostingEntry; var IsHandled: Boolean)
    begin
        IsHandled := true;
    end;
}
