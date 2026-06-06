codeunit 51201 "Work Listener"
{
    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Work Engine", 'OnAfterDoWork', '', true, true)]
    local procedure HandleAfterDoWork()
    begin
    end;

    [EventSubscriber(ObjectType::Codeunit, Codeunit::"Sales-Post", 'OnAfterPostSalesDoc', '', true, true)]
    local procedure HandleBaseAppEvent()
    begin
    end;
}
